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
mod diagnostics;
mod gizmo_ops;
mod lifecycle;
mod meshing;
mod outliner;
mod overlay;

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

/// How fast a hold-and-drag moves the brush numbers.
///
/// Radius is in log space so the same drag is the same proportion at any size;
/// about 250 px covers the whole range, which is a comfortable sweep.
const RADIUS_PER_PIXEL: f32 = 0.006;
const STRENGTH_PER_PIXEL: f32 = 0.002;

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

/// Roughly what a consumer resin printer resolves in XY, in millimetres.
///
/// A reference point for the detail advice, not a limit on anything. Mono LCD
/// panels in the 8K class land near this; the exact figure varies by machine
/// and it is used only to say "you are at or below this", which does not need
/// to be exact.
const RESIN_XY_MM: f32 = 0.035;

/// A typical filament nozzle's extrusion width, in millimetres. The other end
/// of the same comparison: detail finer than this cannot be printed by FDM at
/// all, so a voxel well under it is being spent on nothing unless the model is
/// bound for resin.
const FDM_LINE_MM: f32 = 0.4;

/// Largest angle a fully tilted pen steers the stroke by.
///
/// Tablets report tilt against their own range, so this is applied to the
/// normalised value rather than trusting a device to report degrees. Sixty
/// degrees is about as far as a pen can be leaned while still drawing.
const MAX_TILT: f32 = std::f32::consts::PI / 3.0;

/// Frame intervals kept for the rate readout. At 60 fps this averages over
/// about a second, which is long enough to be steady and short enough to react.
const FRAME_HISTORY: usize = 60;

/// How many pointer moves are kept for the next frame before they are handled
/// without one. See [`Hani::pointer_in`].
const MOST_MOVES_KEPT: usize = 64;

/// How far the pointer may travel during a right press and still count as a
/// click rather than an orbit.
///
/// Right drag orbits, so this is the whole difficulty of putting a menu on right
/// click: too tight and a click that wobbles a pixel opens nothing, too loose
/// and a small deliberate orbit opens a menu over the model. Four pixels and a
/// quarter second are a starting point to tune by feel, which is why both are
/// named.
/// The narrowest a cut region may be, in voxels, before it is refused.
///
/// Two, because one is not enough: a region exactly one voxel across lands on a
/// row of voxel centres or falls between two rows depending on where the
/// lattice sits under it, so it removes material in some places and not others
/// for reasons the user cannot see. Two guarantees at least one centre across
/// the narrow direction wherever the region happens to fall.
const MIN_CUT_WIDTH_VOXELS: f32 = 2.0;

/// Bricks a cut may walk looking for loose pieces before it gives up.
///
/// **Sized from a time budget rather than from a memory one.**
/// `Document::split_plan` measures 13 to 17 microseconds per brick -- 423
/// bricks in 7.2 ms, 7,665 in 103 ms, close to linear -- so a brick ceiling IS
/// a time ceiling once the slope is known. At 14 us a brick, 1,200 bricks is
/// about one 16 ms frame, which is the right order for something appended to a
/// status line.
///
/// The number this replaced was 24,000, chosen to sound like "a large model".
/// At the measured slope that is roughly 320 milliseconds -- twenty frames --
/// spent deciding whether to add five words to a sentence, on top of a cut that
/// costs five.
const MOST_BRICKS_TO_WALK: usize = 1_200;

const CLICK_SLOP_PX: f32 = 4.0;
const CLICK_MS: u128 = 250;

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

    /// A pointer event, counted into the frame being timed. A stroke is
    /// noted on both sides of it, so that the press and the release are in a
    /// stroke's frames with the moves between them. See `app/phases.rs`.
    ///
    /// **A move waits for the frame.** A mouse reports five hundred or a
    /// thousand times a second and a pen two hundred, and a frame is drawn
    /// sixty times: several moves arrive between two frames, and one move of
    /// the pointer has been seen to arrive as three events at the same place.
    /// Each used to be a whole step of a stroke: its stamps, then a remesh,
    /// then a second ray march for the ring, then the ring. Only the stamps
    /// are owed to each move. So moves are kept as they come in, and handled
    /// together by [`Hani::handle_moves`] before the next frame or before
    /// anything else is acted on, whichever is first: every move's stamps in
    /// order, and one remesh, one pick and one ring for the lot.
    fn pointer_in(&mut self, event: PointerEvent) {
        self.phases.count(|frame| frame.events += 1);
        if matches!(event, PointerEvent::Moved { .. }) {
            self.moves.push(event);
            // Frames stop coming to a window nothing is shown of, and moves
            // may not. They are not kept without limit.
            if self.moves.len() >= MOST_MOVES_KEPT {
                self.handle_moves();
            }
            return;
        }
        self.handle_moves();
        self.note_a_stroke();
        self.on_pointer(event);
        self.note_a_stroke();
    }

    /// Handle the moves kept since the last frame. See [`Hani::pointer_in`].
    fn handle_moves(&mut self) {
        if self.moves.is_empty() {
            return;
        }
        let mut moves = std::mem::take(&mut self.moves);
        self.note_a_stroke();
        let last = moves.len() - 1;
        for (index, event) in moves.drain(..).enumerate() {
            self.batching = index < last;
            // The pen as it was at this move. See `app/pen.rs`.
            self.pen_at_move(index);
            self.on_pointer(event);
        }
        self.pen_after_moves();
        self.batching = false;
        self.note_a_stroke();
        // The allocation goes back for the next frame's moves.
        self.moves = moves;
    }

    /// Say that a stroke is down in the frame being timed.
    fn note_a_stroke(&self) {
        if matches!(
            self.drag.map(|drag| drag.kind),
            Some(DragKind::Sculpt(_)) | Some(DragKind::Masking)
        ) {
            self.phases.count(|frame| frame.stroke = true);
        }
    }

    /// Count the stamps just laid into the frame being timed.
    fn count_stamps(&self) {
        let stamps = self.perf.stamps as u32;
        self.phases.count(|frame| frame.stamps += stamps);
    }

    /// Keep the brush a constant fraction of the model when asked to.
    ///
    /// ZBrush calls this Dynamic. Without it a brush tuned on a 60 mm ball is
    /// the wrong size the moment the model is resampled or grows.
    ///
    /// **This must never run because the SELECTION changed.** It exists to
    /// survive a *resample*, where the geometry moved under a fixed brush;
    /// choosing a different body changes no geometry at all. Once
    /// `model_radius` means the active body's, clicking a 5 mm rivet beside a
    /// 200 mm bust with Dynamic on would take a 5 mm brush to `5 * 0.025 =
    /// 0.125`, clamped up to [`MIN_RADIUS_MM`], and clicking back would take it
    /// to `0.25 * 40 = 10 mm` -- the brush doubled and the user never touched
    /// the slider. Worse, the press that resized it is the same press that
    /// selects, so the next press carves at a radius they never chose.
    /// Selection changes go through [`Hani::refresh_model_radius`] instead,
    /// which re-measures for the camera and the mirror plane and leaves the
    /// brush alone.
    fn rescale_radius(&mut self, previous_model_radius: f32) {
        if !self.dynamic_radius || previous_model_radius <= 0.0 || self.model_radius <= 0.0 {
            return;
        }
        let factor = self.model_radius / previous_model_radius;
        if (factor - 1.0).abs() > 1.0e-4 {
            self.brush.radius =
                (self.brush.radius * factor).clamp(MIN_RADIUS_MM, self.max_radius());
        }
    }

    /// The largest brush radius that is usable at the current voxel size.
    ///
    /// The lower of the millimetre ceiling and the voxel one. See
    /// [`MAX_RADIUS_VOXELS`] for why the second exists: at a resin voxel the
    /// millimetre ceiling alone reaches a brush that takes a quarter of a
    /// second per stamp.
    pub(crate) fn max_radius(&self) -> f32 {
        MAX_RADIUS_MM.min(MAX_RADIUS_VOXELS * self.doc.voxel_size()).max(MIN_RADIUS_MM)
    }

    /// Open the undo recorder on the body this stroke is about to write to, if
    /// a press has not already opened it.
    ///
    /// **This is called where the edit happens and not where the button goes
    /// down, and that ordering is the whole point.** Recording used to open in
    /// the `Pressed` arm before anything had been raycast, which with more than
    /// one body means opening it on the wrong one -- and `record_for_undo` does
    /// nothing at all when the volume it is called on has no recorder, so the
    /// carve would land, no history entry would be pushed, `unsaved` would stay
    /// false, and quitting would not even raise the discard prompt. One press,
    /// unrecoverable work.
    ///
    /// Lazy rather than moved into the press for one behaviour worth keeping: a
    /// press that starts off the model and drags onto it sculpts, and there is
    /// no body to open a recorder on until it arrives.
    fn arm_recorder(&mut self) {
        let volume = self.doc.active_volume_mut();
        if !volume.is_recording() {
            volume.begin_stroke();
        }
    }

    /// Apply the brush along the stroke path up to the point under the cursor.
    ///
    /// The stroke walks from its previous stamp to the new one at a fixed
    /// spacing, so a fast drag lays a continuous cut instead of a dotted trail.
    /// The stamps are applied one after another rather than batched, because
    /// each one has to see the field the previous one left behind.
    ///
    /// Move is the exception, and takes none of that path. It locked the field
    /// when the button went down and re-warps that same copy by the whole drag
    /// so far, so it wants the raw pointer position and one pass, not a trail
    /// of stamps that each build on the last. See `MoveStroke`.
    /// Index of a brush in the per-kind strength table.
    fn strength_slot(kind: BrushKind) -> usize {
        BrushKind::ALL.iter().position(|candidate| *candidate == kind).unwrap_or(0)
    }

    /// Where the LIVE tool's strength lives in the table.
    ///
    /// The mask has a slot of its own; the cut has none and borrows the
    /// brush's, which is correct because arming the cut must not disturb what
    /// the brush was set to and a cut has no strength to remember.
    fn live_strength_slot(&self) -> usize {
        match self.tool {
            Tool::Mask => MASK_STRENGTH_SLOT,
            Tool::Sculpt | Tool::Cut | Tool::Transform => Self::strength_slot(self.brush.kind),
        }
    }

    /// The highest strength the live tool allows. See [`MAX_MASK_STRENGTH`].
    pub(crate) fn max_strength(&self) -> f32 {
        match self.tool {
            Tool::Mask => MAX_MASK_STRENGTH,
            Tool::Sculpt | Tool::Cut | Tool::Transform => MAX_STRENGTH,
        }
    }

    /// What the right-click menu's heading should read.
    ///
    /// The menu is contextual on the live tool, so in mask mode it has to say
    /// MASK -- and it has to say BLUR while shift is held, which is the same
    /// live substitution the strip makes and the reason this is not just
    /// `self.tool`.
    pub(crate) fn live_tool_label(&self) -> String {
        match (self.tool, self.shift) {
            (Tool::Mask, true) => "MASK — BLUR".to_string(),
            (Tool::Mask, false) => "MASK".to_string(),
            // Shape-aware, through the same live substitution the mask's BLUR
            // uses: the tool card has to say what the gesture in progress is
            // going to do, and "PLANE CUT" over a lasso is a lie about which
            // material is about to go. Before a drag starts there is no shape
            // yet and it says what an unqualified cut is.
            // Alt keeps both sides, and the card says so WHILE it is held,
            // for the reason shift's preview is live: a verb read from a key
            // at the release has to be visible before the release.
            (Tool::Cut, _) if self.parting_cut() => "PART — KEEP BOTH".to_string(),
            // See `app/ring_cut.rs`.
            (Tool::Cut, _) if self.rings() => "RING CUT".to_string(),
            (Tool::Cut, _) => self.live_cut_shape().label().to_string(),
            (Tool::Transform, _) => "MOVE".to_string(),
            (Tool::Sculpt, _) => self.effective_brush().kind.label().to_uppercase(),
        }
    }

    /// How the path drawn so far would be read, for the label and the preview.
    ///
    /// [`cut::CutShape::Line`] before anything is drawn, which is both the safe
    /// default and the true one: a cut that has not been dragged yet is the cut
    /// this application has always had.
    fn live_cut_shape(&self) -> cut::CutShape {
        cut::read_stroke(&self.cut_path, CLICK_SLOP_PX)
            .map_or(cut::CutShape::Line, |gesture| gesture.shape)
    }

    /// Put the live strength away and bring out the one the incoming tool or
    /// brush was last set to.
    ///
    /// One function and two callers -- a brush change and a tool change --
    /// because the swap has to happen on both and two copies of it would drift.
    ///
    /// **Deliberately does not clamp on the way out.** `MAX_STRENGTH` is the
    /// slider's ceiling and not a hard one: `BrushKind::Move`'s own default is
    /// 1.0, deliberately above it, because for Move strength is the fraction of
    /// the drag the surface follows and a grab tool at 0.15 crawls. Clamping
    /// here would silently cap Move at 0.80 on every tool change, which
    /// `each_brush_remembers_its_own_strength` catches. Nothing needs the
    /// clamp anyway: the mask's 1.0 lives in its own slot and is never loaded
    /// into a brush's.
    fn swap_strength(&mut self, into: impl FnOnce(&mut Self)) {
        self.strengths[self.live_strength_slot()] = self.brush.strength;
        into(self);
        self.brush.strength = self.strengths[self.live_strength_slot()];
    }

    /// Where the pointer is, in the plane through `through` that faces the
    /// camera.
    ///
    /// This is what makes a grab feel like a grab. Raycasting the pointer onto
    /// the surface instead gives a target that crawls ALONG the form: drag
    /// sideways across a ball and the hit point slides around its curve, so the
    /// vector from the grab point stays short and keeps turning, and the result
    /// is a smear rather than a pull. It is also a feedback loop, because the
    /// surface being raycast is the one currently being deformed.
    ///
    /// Projecting into the view plane instead means the surface follows the
    /// cursor the way the hand expects, in the plane of the screen, and it
    /// keeps working when the cursor is dragged off the model entirely -- which
    /// is exactly when a grab is most useful.
    fn view_plane_point(&self, pixel: Vec2, through: Vec3) -> Vec3 {
        let (origin, ray) = self.ray_through(pixel);
        let Some(facing) = (self.camera.target - self.camera.eye()).try_normalize() else {
            return through;
        };
        let denominator = ray.dot(facing);
        // The ray is parallel to the plane, which cannot happen for a pointer
        // inside the viewport but is cheap to refuse.
        if denominator.abs() < 1.0e-6 {
            return through;
        }
        origin + ray * ((through - origin).dot(facing) / denominator)
    }

    /// One event of a Move gesture.
    ///
    /// Kept apart from `sculpt_to` because Move shares almost none of it: no
    /// stroke interpolation, no per-stamp normals, no trail of stamps building
    /// on each other. It locks the field once and re-warps that copy by the
    /// whole drag so far.
    fn move_to(&mut self, pixel: Vec2, start: bool) {
        let brush = self.effective_brush();
        let pressure = self.tablet.stamp_pressure(self.pressure_enabled, self.pressure_curve);
        let started = Instant::now();

        // The anchor is the one thing that needs the surface, and it is taken
        // once. After that the gesture is about the pointer.
        if start || !self.move_stroke.is_active() {
            let Some(point) = self.surface_under(pixel) else {
                return;
            };
            // Past the last early return, so the recorder opens on the body
            // that is about to be warped and not a moment sooner.
            self.arm_recorder();
            self.move_stroke.begin(
                self.doc.active_volume(),
                &brush,
                point,
                self.framed_symmetry(),
                MIRROR_CENTRE,
            );
            self.move_grab = Some(point);
        }
        let Some(grab) = self.move_grab else {
            return;
        };

        let target = self.view_plane_point(pixel, grab);
        self.move_stroke.drag_to(self.doc.active_volume_mut(), target, pressure);

        // Re-anchor once this lock has warped as far as it safely can, and let
        // the drag carry on from where the material now is.
        //
        // Without this a gesture stops dead at `Brush::max_drag`, which is a
        // quarter of the fold threshold and only a millimetre and a half for
        // the default 3 mm brush -- the surface simply refuses to follow the
        // cursor any further, which is exactly the "it does something, but not
        // what I expected" complaint. The cap itself cannot just be raised: past
        // the fold threshold a domain warp turns the field back through itself.
        // Chaining fold-safe warps gives unlimited reach without ever crossing
        // it. ZBrush can drag as far as it likes because it moves mesh vertices,
        // which stretch; a field has to be re-locked instead.
        //
        // Safe inside one stroke because `record_for_undo` captures a brick the
        // first time it is touched and not again, so a chain of locks is still
        // one undo entry.
        if self.move_stroke.is_at_the_limit() {
            let carried = grab + self.move_stroke.applied();
            self.move_stroke.begin(
                self.doc.active_volume(),
                &brush,
                carried,
                self.framed_symmetry(),
                MIRROR_CENTRE,
            );
            self.move_grab = Some(carried);
        }

        self.perf.stamps = 1;
        self.count_stamps();
        self.perf.pressure = pressure;
        self.perf.edit_ms = started.elapsed().as_secs_f32() * 1000.0;
        self.remesh_dirty();
        self.hover = Some(grab);
        self.refresh_overlay();
    }

    /// What a left press means while the sculpt tool is live.
    ///
    /// Lifted out of the press dispatch when the cut and the mask collapsed
    /// into one match on [`Tool`]: three arms of body-resolution reasoning
    /// nested inside a `match` on the tool inside a `match` on the button is
    /// three levels deep and reads as none of them.
    ///
    /// **The body is resolved from the raycast BEFORE anything opens a
    /// recorder**; see `arm_recorder`. A press over a body that is not the
    /// active one selects it and carves nothing, and the press after that
    /// sculpts -- the same rule Photoshop's layer panel has, applied to the
    /// model itself so that direct manipulation acts on what is drawn. A press
    /// over a HIDDEN body picks nothing, because `Document::pick` never marches
    /// one.
    fn sculpt_press(&mut self, position: Vec2) -> DragKind {
        match self.pick(position) {
            Some((body, _)) if body != self.doc.active() => {
                self.select_body(body);
                DragKind::Selecting
            }
            Some(_) => {
                self.warn_if_masked(position);
                DragKind::Sculpt(self.stroke_direction())
            }
            // Nothing drawn under the pointer. That is normally a press beside
            // the model, which is allowed to become a stroke the moment it is
            // dragged onto one -- but not when the body it would carve is itself
            // hidden. See `active_is_drawn`: the stroke's own raycast does not
            // consult the eye, so without this the press carves a body nobody
            // can see. Saying which body is the honest half; a press that
            // silently does nothing is its own puzzle.
            //
            // **And it must say which of the two hid it.** Under solo, the
            // active body can be undrawn with its own eye plainly open --
            // clicking a row outside the scope selects it, and `select_body`
            // does not veto that, because a view mode never vetoes a structural
            // operation. "Show it before sculpting it" would then be advice for
            // a state the user is not in, pointing at an eye that is already on.
            None if !self.active_is_drawn() => {
                let active = self.doc.active();
                let name =
                    self.doc.node(active).map_or("the active body", |node| node.name.as_str());
                self.status = if self.in_solo_scope(active) {
                    format!("{name} is hidden — show it before sculpting it")
                } else {
                    format!("{name} is outside the solo scope — escape leaves solo")
                };
                DragKind::Refused
            }
            None => DragKind::Sculpt(self.stroke_direction()),
        }
    }

    /// Say so when a sculpt press lands on protected material.
    ///
    /// **The one thing that stops a mask reading as a broken brush.** A stroke
    /// over a fully protected surface leaves the field bit-identical -- that is
    /// the whole point of the mask -- and with nothing on screen saying why, the
    /// brush looks broken. The mask was very possibly painted minutes ago, or on
    /// another body, so there is nothing under the cursor for the user to
    /// connect it to. The cut has the same line for the same reason; see
    /// `finish_cut`.
    ///
    /// Answered at the press only, from the ONE voxel under the cursor, and
    /// both halves are the point: a per-stamp check would be a map probe in the
    /// stroke loop, and a claim about the whole footprint would need a pass over
    /// it. So it is deliberately narrow -- "the surface you pressed on is fully
    /// protected" -- and stays silent about a stroke that is merely weakened,
    /// which the ring and the tint report instead.
    fn warn_if_masked(&mut self, pixel: Vec2) {
        let volume = self.doc.active_volume();
        if volume.mask().is_free() {
            return;
        }
        let Some(point) = self.surface_under(pixel) else {
            return;
        };
        let cell = (point / self.doc.voxel_size()).round().as_ivec3();
        if self.doc.active_volume().mask().at(cell) != hani_core::PROTECTED {
            return;
        }
        let active = self.doc.active();
        let name = self.doc.node(active).map_or("this body", |node| node.name.as_str()).to_string();
        self.status = format!("the mask on {name} protects this — m, then ctrl drag, unmasks");
    }

    /// Which way a mask stroke is going, and whether it is blurring instead.
    ///
    /// **Its own computation, and NOT `stroke_direction`.** That one gates on
    /// `brush.kind.is_directional()`, which answers false for Smooth, Flatten
    /// and Move -- so reusing it would make ctrl+drag in mask mode *mask*
    /// instead of unmasking for three of the seven brushes, with nothing on
    /// screen to say which three. This is two lines that never consult
    /// `brush.kind`, and the stylus eraser end falls out unmasking for free.
    ///
    /// Shift outranks the pair, which mirrors the field side: shift is Smooth
    /// there whatever the invert modifier says, and it is Blur here.
    fn mask_op(&self) -> MaskOp {
        if self.shift {
            return MaskOp::Blur;
        }
        // Either modifier, and they do NOT compound -- holding both is still one
        // inversion, exactly as `stroke_direction` reads them.
        let unmask = (self.control || self.alt) != self.eraser_in_use();
        if unmask { MaskOp::Lower } else { MaskOp::Raise }
    }

    /// Paint the mask along the stroke path up to the point under the cursor.
    ///
    /// The sculpt path's twin, and deliberately its shape rather than a new
    /// one: the same [`Stroke`] walker at the same spacing, the same lazy
    /// recorder, the same pressure sample once per event. Masking is per body
    /// and lands on the ACTIVE one, so it asks `surface_under` rather than
    /// picking -- the same rule a live sculpt stroke follows once it has begun.
    ///
    /// Move has no analogue here and needs none: the mask is a value at a
    /// lattice cell, not a shape to warp, and there is nothing for a locked copy
    /// to be a copy of.
    fn mask_to(&mut self, pixel: Vec2, start: bool) {
        let Some(point) = self.surface_under(pixel) else {
            // The cursor ran off the model. The stroke stays live so coming back
            // onto it continues rather than restarting, but nothing is painted
            // in mid air.
            return;
        };

        // Past the early return, for the same reason the sculpt path is: the
        // recorder opens on the body about to be painted and only once there is
        // something to paint. `end_stroke` then produces ONE entry carrying the
        // mask bricks, which is what makes a mask-only gesture undoable and
        // what sets `unsaved`.
        self.arm_recorder();

        let started = Instant::now();
        self.stamp_centres.clear();
        if start {
            self.stroke.begin(point, &mut self.stamp_centres);
            // A stroke's own record of what it gave: see `mask_stroke`.
            self.mask_stroke.begin();
        } else {
            let spacing = self.brush.spacing(self.doc.voxel_size());
            self.stroke.advance(point, spacing, &mut self.stamp_centres);
        }

        let pressure = self.tablet.stamp_pressure(self.pressure_enabled, self.pressure_curve);
        let op = self.mask_op();
        // `self.brush` and not `effective_brush`: shift means Blur here, and
        // asking for the effective brush would hand the mask painter Smooth's
        // kind, which it does not read, and Smooth's strength, which it must
        // not -- the mask keeps its own.
        let brush = self.brush;
        let lean = self.pen_lean();
        let tangent = self.stroke.direction().unwrap_or(Vec3::ZERO);

        for index in 0..self.stamp_centres.len() {
            let centre = self.stamp_centres[index];
            let normal = lean_normal(self.doc.active_volume().gradient_world(centre), lean);
            // The direction a mask stamp carries is never read -- protection has
            // no add and subtract, `op` says which way this goes -- so `Add` is
            // the value that cannot be wrong rather than a choice.
            let stamp = Stamp::new(centre, normal, BrushDirection::Add)
                .with_pressure(pressure)
                .with_tangent(tangent);
            // By the stroke, so it fades to nothing at the radius rather
            // than settling solid nearly to it: `hani_core::mask_stroke`.
            let symmetry = self.framed_symmetry();
            self.mask_stroke.stamp(
                &brush,
                self.doc.active_volume_mut(),
                &stamp,
                op,
                symmetry,
                MIRROR_CENTRE,
                &mut self.brush_scratch,
            );
        }

        self.perf.stamps = self.stamp_centres.len();
        self.count_stamps();
        self.perf.pressure = pressure;
        self.perf.edit_ms = started.elapsed().as_secs_f32() * 1000.0;
        self.remesh_dirty();
    }

    /// Paint the chosen filament slot along the stroke path up to the point
    /// under the cursor.
    ///
    /// `mask_to`'s twin, and the same shape for the same reasons: the same
    /// stroke walker at the same spacing, the same lazy recorder, and the
    /// active body only. The inverted direction is the eraser, which writes
    /// [`hani_core::colour::UNPAINTED`]; there is no third operation.
    fn paint_to(&mut self, pixel: Vec2, direction: BrushDirection, start: bool) {
        let Some(point) = self.surface_under(pixel) else {
            return;
        };
        // The eyedropper, before anything is armed: taking a colour is not
        // an edit. See `app/painting.rs`.
        if self.eyedropper_takes(point, start) {
            return;
        }
        // A hidden or locked layer takes no paint. See `app/layers.rs`.
        if self.layer_refuses_paint() {
            return;
        }
        // The airbrush sprays by the frame, not by the stamp. See
        // `app/airbrush.rs`.
        if self.airbrushing() {
            self.airbrush_to(pixel, direction, start);
            return;
        }
        // A decal is dabbed, not stroked. See `app/decal.rs`.
        if self.dabbing() {
            self.decal_to(pixel, start);
            return;
        }
        self.arm_recorder();

        let started = Instant::now();
        self.stamp_centres.clear();
        if start {
            self.stroke.begin(point, &mut self.stamp_centres);
        } else {
            let spacing = self.brush.spacing(self.doc.voxel_size());
            self.stroke.advance(point, spacing, &mut self.stamp_centres);
        }

        // A fill is not a stroke: the paint is poured where the pointer is.
        // See `app/painting.rs`.
        if self.touch.kind == crate::message::PaintTouch::Fill {
            self.pour_colour(point, direction);
            self.remesh_dirty();
            return;
        }
        let lean = self.pen_lean();
        let tangent = self.stroke.direction().unwrap_or(Vec3::ZERO);
        // Pressure is sampled so the readout stays honest; the brush does not
        // read it for strength, because a slot has no half. See
        // `BrushKind::Paint`. It can size the brush: see `app/studio.rs`.
        let pressure = self.tablet.stamp_pressure(self.pressure_enabled, self.pressure_curve);
        let brush = self.studio.sized(self.brush, pressure);

        for index in 0..self.stamp_centres.len() {
            let centre = self.stamp_centres[index];
            let normal = lean_normal(self.doc.active_volume().gradient_world(centre), lean);
            let stamp =
                Stamp::new(centre, normal, direction).with_pressure(pressure).with_tangent(tangent);
            // A loaded brush is upstream's stamp, reached through the coat;
            // a drybrush or a sponge is the same stamp asked one more
            // question per voxel. See `app/paints.rs`.
            let coat = self.coat_for(centre, start && index == 0);
            // A stamp of the colour brush. See `app/painting.rs`.
            self.lay_colour(&brush, &stamp, coat, direction, start && index == 0);
        }

        self.perf.stamps = self.stamp_centres.len();
        self.count_stamps();
        self.perf.pressure = pressure;
        self.perf.edit_ms = started.elapsed().as_secs_f32() * 1000.0;
        self.remesh_dirty();
    }

    fn sculpt_to(&mut self, pixel: Vec2, direction: BrushDirection, start: bool) {
        // A shape or a cable being added is a gesture of its own too. See
        // `app/adding.rs`.
        if self.inserting() {
            self.add_to(pixel, direction, start);
            return;
        }
        // A stamp is a gesture of its own and not a stroke of any brush.
        // See `app/studio.rs`.
        if self.studio.stamping() {
            self.stamp_to(pixel, direction, start);
            return;
        }
        // Move is handled before anything else, because it wants the pointer
        // rather than the surface under the pointer, and it must keep working
        // once the cursor has been dragged off the model.
        if self.effective_brush().kind == BrushKind::Move {
            self.move_to(pixel, start);
            return;
        }
        // After Move, which wants the pointer itself: every stamping brush
        // can be led. See `app/studio.rs`.
        let pixel = self.studio.follow(pixel, start);
        // Paint has no field path either. `effective_brush` so that shift
        // still smooths with the paint brush selected, as it does with every
        // other.
        if self.effective_brush().kind == BrushKind::Paint {
            self.paint_to(pixel, direction, start);
            return;
        }

        let Some(point) = self.surface_under(pixel) else {
            // The cursor ran off the model. The stroke stays live so coming
            // back onto it continues rather than restarting, but nothing is
            // stamped in mid air.
            return;
        };

        // Past the early return, so the recorder opens on the body that is
        // about to be carved and only once there is something to carve. See
        // `arm_recorder`.
        self.arm_recorder();
        // A stroke that lays one layer is aimed at the surface as it was
        // before the stroke, and faces the way that did. See
        // `app/studio.rs`.
        let point = self.layer_aim(pixel, point);

        let started = Instant::now();
        self.stamp_centres.clear();
        if start {
            self.stroke.begin(point, &mut self.stamp_centres);
        } else {
            // Upstream's spacing, or the studio's for a brush set down in
            // dabs. See `app/studio.rs`.
            let spacing = self.stroke_spacing();
            self.stroke.advance(point, spacing, &mut self.stamp_centres);
        }

        // Sampled once for the whole event rather than per stamp: the pen has
        // not moved between the stamps that one pointer event interpolates, so
        // re-reading it would only add jitter.
        let pressure = self.tablet.stamp_pressure(self.pressure_enabled, self.pressure_curve);
        let brush = self.effective_brush();

        {
            // A locked copy of the field is only good while nothing else writes
            // over it, and this is about to.
            self.move_stroke.end();
            self.move_grab = None;

            // Not for a brush that levels to a plane: `app/pen_strokes.rs`.
            let lean = self.stroke_lean(self.pen_lean());
            // Which way the drag is going, for the patterns that comb. Zero
            // until the stroke has moved far enough to have a direction, which
            // the pattern copes with by picking any direction across the
            // surface.
            let tangent = self.stroke.direction().unwrap_or(Vec3::ZERO);

            for index in 0..self.stamp_centres.len() {
                let centre = self.stamp_centres[index];
                // Take the normal from the field at each stamp rather than
                // reusing the one from the raycast, so a stroke curving around
                // a form stays oriented to the surface it is actually on.
                // Leaning the pen rotates the direction the brush pushes in,
                // which steers every brush at once because they all read this
                // normal.
                let normal = lean_normal(self.stamp_facing(centre), lean);
                let stamp = Stamp::new(centre, normal, direction)
                    .with_pressure(pressure)
                    .with_tangent(tangent);
                // Upstream's stamp, or the studio's generic brush. See
                // `app/studio.rs`.
                self.stamp_field(brush, &stamp);
            }
            self.perf.stamps = self.stamp_centres.len();
            self.count_stamps();
        }
        // Paint carried onto the surface the stamps made: `app/paint_carry.rs`.
        self.carry_paint_after_stamps(brush, pressure);
        self.perf.pressure = pressure;
        self.perf.edit_ms = started.elapsed().as_secs_f32() * 1000.0;

        self.remesh_dirty();
    }

    /// The brush a stamp actually runs with.
    ///
    /// Holding shift smooths, which is the convention every sculpting tool
    /// uses and the single biggest ergonomic win available here. Shift already
    /// modified right drag into a pan, and this is left drag, so there is no
    /// clash.
    ///
    /// Deliberately does NOT swap `self.brush.kind` and swap it back: nothing
    /// then has to notice a key being released while the window is unfocused,
    /// and the tool strip does not flicker between two highlights during a
    /// stroke. The selection is never touched, so there is nothing to restore.
    ///
    /// **Shift over the paint brush smooths at Smooth's OWN remembered
    /// strength**, not the paint brush's. Paint's strength is a fixed 1.0 the
    /// panel hides -- a slot has no half -- and carrying it into Smooth meant
    /// shift-drag smoothed at 2.5x Smooth's default and above the slider's
    /// ceiling, with no control on screen to say so. Every other brush keeps
    /// its own strength under shift, as before.
    fn effective_brush(&self) -> Brush {
        if !self.shift {
            return self.brush;
        }
        let strength = if self.brush.kind == BrushKind::Paint {
            self.strengths[Self::strength_slot(BrushKind::Smooth)]
        } else {
            self.brush.strength
        };
        Brush { kind: BrushKind::Smooth, strength, ..self.brush }
    }

    /// Direction for a new stroke, honouring the invert modifier, the eraser
    /// end of the stylus, and the fact that some brushes have no opposite.
    ///
    /// The two inverts combine rather than override: holding the modifier while
    /// using the eraser gives back the additive brush, which is the same
    /// behaviour every drawing application has.
    fn stroke_direction(&self) -> BrushDirection {
        // Either modifier inverts. Alt is what ZBrush and Nomad both use, and
        // control is what this had first; keeping both costs nothing and means
        // neither habit is wrong. They do NOT compound -- holding both is still
        // one inversion, because a user pressing both means "invert", not
        // "invert twice".
        let modifier = self.control || self.alt;
        let inverted = modifier != self.eraser_in_use();
        if inverted && self.brush.kind.is_directional() {
            BrushDirection::Subtract
        } else {
            BrushDirection::Add
        }
    }

    /// Whether the eraser end of the stylus is the one in range.
    fn eraser_in_use(&self) -> bool {
        let pen = self.tablet.state();
        pen.in_proximity && pen.eraser
    }

    /// The world space lean of the pen, as a vector whose length is the tilt
    /// angle in radians.
    ///
    /// Tilt arrives in the tablet's own frame, which lines up with the screen,
    /// so it has to be carried into world space through the camera basis before
    /// it can steer anything.
    fn pen_lean(&self) -> Vec3 {
        if !self.tilt_enabled {
            return Vec3::ZERO;
        }
        let pen = self.tablet.state();
        if !pen.in_proximity {
            return Vec3::ZERO;
        }

        let magnitude = pen.tilt.length().min(1.0);
        if magnitude < 1.0e-4 {
            return Vec3::ZERO;
        }

        // Screen y grows downward and the camera's up axis grows upward, and a
        // positive tilt on that axis means the pen is leaning toward the user,
        // which is toward the bottom of the screen. Hence the subtraction.
        let direction =
            (self.camera.right() * pen.tilt.x - self.camera.up() * pen.tilt.y).normalize_or_zero();
        direction * (magnitude * MAX_TILT)
    }

    /// The narrowest width of a screen-space hull, in voxels, at the depth the
    /// camera is focused on.
    ///
    /// The narrowest and not the average: a long thin lens is wide in one
    /// direction and vanishing in the other, and it is the vanishing one that
    /// decides whether the cut lands on any voxel centres at all.
    ///
    /// Found by rotating-callipers over the hull's edges -- for a convex
    /// polygon the minimum width is always across some edge, so the furthest
    /// vertex from each edge in turn is the whole search. At most sixteen edges
    /// by sixteen vertices, once per gesture.
    fn hull_width_in_voxels(&self, hull: &[Vec2]) -> f32 {
        let voxel_size = self.doc.voxel_size();
        if hull.len() < 3 || voxel_size <= 0.0 {
            return f32::INFINITY;
        }
        // Millimetres per pixel where the camera is looking, which is where the
        // preview is drawn and where the user believes the shape is.
        let target = self.camera.target;
        let scale = {
            let centre = self.viewport_size * 0.5;
            let (_, straight) = self.ray_through(centre);
            let (_, offset) = self.ray_through(centre + Vec2::X);
            let depth = (target - self.camera.eye()).length();
            (straight * depth).distance(offset * depth)
        };

        let mut narrowest = f32::INFINITY;
        for index in 0..hull.len() {
            let from = hull[index];
            let to = hull[(index + 1) % hull.len()];
            let Some(along) = (to - from).try_normalize() else {
                continue;
            };
            // Perpendicular distance from the edge to the furthest vertex.
            let across = hull
                .iter()
                .map(|point| ((*point - from).x * along.y - (*point - from).y * along.x).abs())
                .fold(0.0, f32::max);
            narrowest = narrowest.min(across);
        }
        narrowest * scale / voxel_size
    }

    /// A far cap for the cutter, so it stops behind what it is cutting off
    /// instead of carrying on through the model.
    ///
    /// **This is the request, in one function.** "Something that makes it
    /// easier to cut off things that are sticking out of the main body" is
    /// unanswerable with an unbounded cutter: circling a spur on the near side
    /// of a scan also takes whatever is behind it, and the only defence is a
    /// hand-painted mask on the far side -- which is the workaround every
    /// comparable tool ships and the one this is meant to remove.
    ///
    /// Returns `None` when there should be no cap, which is a real outcome
    /// rather than a failure. Three ways to get it, and they are not the same:
    ///
    /// * A straight drag. One plane is an infinite half-space by definition;
    ///   that is what the plane cut has always been and what
    ///   `a_cut_passes_through_the_whole_model` asserts of it.
    /// * Nothing was hit, so there is no depth to measure from.
    /// * There is nothing behind worth sparing -- and `report` is set, so the
    ///   status can say so rather than let the tool look clever.
    ///
    /// # Why the cap is clamped against the NEXT surface
    ///
    /// Placing it one band past the deepest exit is the obvious rule and it is
    /// wrong. At a 0.25 mm voxel a band is 0.75 mm, so a wall half a millimetre
    /// behind a spur is thinned from the front by a quarter of a millimetre
    /// with a sharp step -- and `is_printable` returns true for the result,
    /// because it counts holes and winding and has no thickness term at all.
    /// So the cap is pulled back to stay a full band clear of whatever is
    /// behind, and where there is not room for both it declines and says so. A
    /// tool that quietly thins the wall behind is worse than one that admits it
    /// cannot help.
    fn depth_cap(
        &self,
        hull: &[Vec2],
        report: &mut Option<&'static str>,
    ) -> Option<(hani_core::ClipPlane, f32)> {
        if hull.len() < 3 {
            return None;
        }
        let eye = self.camera.eye();
        let facing = (self.camera.target - eye).try_normalize()?;
        let reach = self.camera.far();

        // Points known to be inside a CONVEX hull: its centroid, and points on
        // the way from the centroid to each vertex. Deliberately not a bounding
        // box grid, which would put rays outside a hull that is not square, and
        // deliberately not the centroid alone -- one ray down the middle is a
        // sample of one, and on a long thin loop it can miss the spur entirely.
        let centroid = hull.iter().copied().sum::<Vec2>() / hull.len() as f32;
        let mut probes = vec![centroid];
        for vertex in hull {
            for fraction in [0.45_f32, 0.8] {
                probes.push(centroid.lerp(*vertex, fraction));
            }
        }

        let bodies = self.drawn_bodies();
        let mut deepest_exit = f32::NEG_INFINITY;
        let mut shallowest_enter = f32::INFINITY;
        let mut nearest_behind = f32::INFINITY;
        for pixel in probes {
            let (origin, ray) = self.ray_through(pixel);
            // Distances come back along each ray from its own origin on the
            // near plane, so they are converted to DEPTH along the view
            // direction before being compared between rays. Without this a ray
            // at the edge of a wide hull reports a longer distance for the same
            // depth and drags the cap backwards -- which is the same class of
            // mistake as measuring the cutter's apex from the near plane.
            let scale = ray.dot(facing);
            if scale <= 0.0 {
                continue;
            }
            let base = (origin - eye).dot(facing);
            for body in &bodies {
                let Some(volume) = self.doc.volume(*body) else {
                    continue;
                };
                let spans = hani_core::first_solid_spans(volume, origin, ray, reach);
                if let Some(span) = spans.first {
                    deepest_exit = deepest_exit.max(base + span.exit * scale);
                    shallowest_enter = shallowest_enter.min(base + span.enter * scale);
                }
                if let Some(next) = spans.next_enter {
                    nearest_behind = nearest_behind.min(base + next * scale);
                }
            }
        }

        if deepest_exit == f32::NEG_INFINITY {
            // Every ray missed. There is no depth here, and the existing "the
            // cut missed the model" outcome is the one that applies.
            return None;
        }

        let margin = hani_core::NARROW_BAND * self.doc.voxel_size();
        let want = deepest_exit + margin;
        if nearest_behind.is_finite() {
            if nearest_behind - deepest_exit < 2.0 * margin {
                *report = Some("nothing behind it to spare — the cut went through");
                return None;
            }
            let limit = nearest_behind - margin;
            let at = want.min(limit);
            return hani_core::ClipPlane::new(eye + facing * at, -facing)
                .map(|plane| (plane, at - shallowest_enter));
        }
        hani_core::ClipPlane::new(eye + facing * want, -facing)
            .map(|plane| (plane, want - shallowest_enter))
    }

    /// How many loose pieces the cut left behind, as a phrase to append to the
    /// status, or nothing.
    ///
    /// **A cut that severs a body leaves two shells inside one `Volume` with
    /// nothing anywhere saying so.** The user gets a model that looks right,
    /// exports as one object, and arrives at the slicer in two pieces -- which
    /// is precisely the failure the research identified in the tools this one
    /// is meant to beat.
    ///
    /// `Document::split_plan` is read-only, parallel and already tested, so
    /// this is a report and not a second connectivity walk. It is specifically
    /// NOT a brick-level walk: `split.rs` measures one finding a single
    /// component on the reference dragon where the voxel walk finds 29, 47, 85
    /// and 182 at four voxel sizes, and fusing parts of a real four-part model.
    ///
    /// # It is expensive, so it is bounded three ways
    ///
    /// `split_plan` is a per-voxel union-find and costs roughly 13 to 17
    /// microseconds per brick -- more, on the bench fixture, than the entire
    /// cut that preceded it. Left unbounded this one status-line suffix is the
    /// most expensive thing on the commit path by a wide margin. So:
    ///
    /// 1. **Only bodies the cut actually changed.** They come from the outcome
    ///    rather than from asking each body whether anything is dirty, which
    ///    would be reading an answer off a set that merely has not been drained
    ///    yet. Walking a body the cutter never reached costs full price and can
    ///    only ever find what was already there before the gesture.
    /// 2. **One budget across all of them**, not one each: eight small bodies
    ///    are eight walks, and a per-body gate lets them through.
    /// 3. **A budget sized in bricks from a time target.** See
    ///    [`MOST_BRICKS_TO_WALK`].
    ///
    /// Going over budget stops and reports what was found so far rather than
    /// reporting nothing: pieces already counted are true, and throwing them
    /// away because a later body was large would hide a real severance behind
    /// an unrelated body's size.
    fn loose_pieces_note(&self, changed: &[NodeId]) -> String {
        let mut pieces = 0usize;
        let mut walked = 0usize;
        for body in changed {
            let Some(volume) = self.doc.volume(*body) else {
                continue;
            };
            // `brick_count` and not `brick_coords().count()`: the second walks
            // the map to answer a question the map already knows.
            walked += volume.brick_count();
            if walked > MOST_BRICKS_TO_WALK {
                break;
            }
            let Some(plan) = self.doc.split_plan(*body) else {
                continue;
            };
            // One part is the body itself and is not news.
            pieces += plan.parts.len().saturating_sub(1);
        }
        match pieces {
            0 => String::new(),
            1 => " — 1 loose piece".to_string(),
            many => format!(" — {many} loose pieces"),
        }
    }

    /// The DRAWN bodies, by node id -- the same set the cut acts on.
    fn drawn_bodies(&self) -> Vec<NodeId> {
        let visible = self.drawn_nodes();
        self.doc
            .nodes()
            .iter()
            .enumerate()
            .filter(|(index, _)| visible.get(*index).copied().unwrap_or(false))
            .map(|(_, node)| node.id)
            .collect()
    }

    /// Build the cutter's planes from a screen-space hull.
    ///
    /// Each edge of the hull becomes a plane containing that edge and the view
    /// direction, so the region removed is the PRISM the polygon sweeps out
    /// going away from the viewer -- the same width front and back. `None` if
    /// any edge is degenerate: all or nothing, because a polygon missing one
    /// side is not a smaller region, it is an open wedge that carries on for
    /// ever.
    ///
    /// The straight drag is the exception and keeps its plane through the eye:
    /// a single half-space has no aperture, so there is nothing for a taper to
    /// widen, and that construction is what ships and what the side-convention
    /// test observes.
    ///
    /// # The eye is `camera.eye()`, and that is a bug fix
    ///
    /// [`OrbitCamera::ray`] returns the point where the ray crosses the NEAR
    /// PLANE, not the eye -- and it must keep doing so, because every pick in
    /// the application starts there and a test pins it. The cut used that point
    /// as its apex, and for a single plane that is correct **by accident**: the
    /// near point lies on the first ray, and the normal is perpendicular to
    /// both rays, so the offset falls out of the dot product exactly.
    ///
    /// With several planes it stops cancelling. Plane `k`'s normal is not
    /// perpendicular to the ray the near point came from, so plane `k` is
    /// displaced by `(P0 - E) . n_k` -- a different amount for every side --
    /// and the pyramid's apex opens into a small polyhedral gap. The bound is
    /// `near()`, which is a hundredth of the camera distance and about 1.5 mm
    /// on a framed 100 mm model, six voxels at 0.25 mm. In practice it is less,
    /// because for a small hull all the rays are nearly parallel and the dot
    /// product is a fraction of that; it grows with the model and with how wide
    /// the gesture is.
    ///
    /// # The sign is measured, not assumed
    ///
    /// For a polygon the winding decides which side each plane removes, and
    /// getting it backwards inverts the cutter: instead of removing the inside
    /// of the loop it removes everything except a pyramid. Rather than reason
    /// about the handedness of the camera basis, the y-flip in the pixel-to-NDC
    /// step and the hull's winding all at once -- three chances to be wrong
    /// with one symptom -- the normals are built to one convention and then
    /// CHECKED against a point known to be inside the region. If the check
    /// fails they are all flipped. A sign that is verified costs one dot
    /// product per plane, once per gesture, and cannot be got wrong by a later
    /// change to any of the three.
    fn cutter_from_hull(&self, hull: &[Vec2]) -> Option<Vec<hani_core::ClipPlane>> {
        let eye = self.camera.eye();

        // Two points name one plane and no region: this is the straight drag,
        // and it keeps its own construction so that the side convention a test
        // observes is untouched.
        if let [from, to] = hull {
            let (_, first) = self.ray_through(*from);
            let (_, second) = self.ray_through(*to);
            return hani_core::ClipPlane::new(eye, second.cross(first)).map(|plane| vec![plane]);
        }
        if hull.len() < 3 {
            return None;
        }

        // **A prism, not a pyramid**, and the difference is the loudest thing
        // about this tool in use. Building each side plane through the EYE
        // makes the removed region the view frustum's own wedge: it widens with
        // distance in exactly the proportion the projection does. At the
        // framing this application picks -- `camera.rs` puts the eye at
        // `radius / sin(fov/2) * 1.15`, about 90 mm for a 30 mm ball -- the
        // front face is 60 mm away and the back 120, so **the exit hole comes
        // out twice the diameter of the entry hole**. Circling a lump takes a
        // cone out of everything behind it.
        //
        // Building them along the VIEW DIRECTION instead makes the sides
        // parallel, so the cutter is the same width all the way through: what a
        // bandsaw does, and what a symmetric pair of cuts needs in order to
        // come out symmetric.
        //
        // The trade is real and worth stating. A pyramid is screen-exact at
        // every depth -- the region removed is precisely the screen area swept,
        // whatever it passes through. A prism is exact only at the focus depth,
        // which is where the preview is drawn; further away the region takes
        // slightly less than the outline appeared to cover, and nearer slightly
        // more. That is the better error to have: it is bounded by the model's
        // own depth rather than growing with it, and it errs toward the
        // recoverable direction on the far side.
        //
        // This is the orthographic-cut behaviour without an orthographic
        // camera. The camera is untouched; only the cutter is parallel.
        let facing = (self.camera.target - eye).try_normalize()?;
        let corner = |pixel: Vec2| {
            let (origin, ray) = self.ray_through(pixel);
            let denominator = ray.dot(facing);
            if denominator.abs() < 1.0e-6 {
                return origin;
            }
            origin + ray * ((self.camera.target - origin).dot(facing) / denominator)
        };

        let mut planes = Vec::with_capacity(hull.len());
        for index in 0..hull.len() {
            let here = corner(hull[index]);
            let next = corner(hull[(index + 1) % hull.len()]);
            planes.push(hani_core::ClipPlane::new(here, (next - here).cross(facing))?);
        }

        // A point genuinely inside the prism: the centroid of a CONVEX polygon
        // is inside it, so the point at the focus depth under the centroid is
        // in the middle of the region.
        let centroid = hull.iter().copied().sum::<Vec2>() / hull.len() as f32;
        let inside = corner(centroid);
        if planes.iter().any(|plane| plane.distance(inside) < 0.0) {
            for plane in &mut planes {
                plane.normal = -plane.normal;
            }
        }
        debug_assert!(
            planes.iter().all(|plane| plane.distance(inside) >= 0.0),
            "the cutter excludes its own centre, so the hull is not convex"
        );
        Some(planes)
    }

    ///
    /// **It cuts every body the user can SEE**, which is [`Document::clip`]'s
    /// whole job: solo narrows the set, a hidden body the line passes over is
    /// left bit-identical, and the whole gesture is one undo entry.
    ///
    /// **And it answers to the mask**, on the same principle: a cut acts on what
    /// is drawn, and a masked region is drawn as protected. A cut a mask blocked
    /// entirely produces the same zero as a cut that missed, so the zero case
    /// below tells the two apart and names the body -- see there for why that
    /// matters more than the count does.
    ///
    /// # The shape comes from the stroke
    ///
    /// [`cut::read_stroke`] decides whether the drag was a line, an open curve
    /// or a closed loop, and hands back a convex polygon in screen pixels. Each
    /// of its edges becomes a plane through the EYE, so the cutter is the
    /// pyramid the polygon sweeps out going away from the viewer -- which is
    /// what makes a screen-space gesture mean something in three dimensions.
    /// A two-point polygon is the straight drag that has always shipped, and it
    /// still produces exactly one plane and one cross product.
    ///
    /// **Which side goes is set by the drag direction**: the material to the
    /// LEFT of the arrow, as drawn on screen, is removed -- so a left to right
    /// drag takes the top half. Drag the other way to keep the other half.
    ///
    /// That sentence is copied from what
    /// `a_left_to_right_drag_removes_a_consistent_side` actually observes, not
    /// from reasoning about the cross product. The sign depends on the ray
    /// order, the handedness of the camera basis and whether the pixel-to-NDC
    /// step flips Y, and getting it backwards means the tool removes the half
    /// the user meant to keep. For a polygon the sign is not reasoned about at
    /// all -- see [`Hani::cutter_from_hull`].
    fn finish_cut(&mut self, from: Vec2) {
        // The path is taken whatever happens next, so no outcome below can
        // leave it to leak into the next gesture.
        let mut path = std::mem::take(&mut self.cut_path);
        if let Some(to) = self.cursor {
            // The release position, which no motion event may have delivered if
            // the pointer stopped moving before the button came up.
            if path.last().is_none_or(|last| *last != to) {
                path.push(to);
            }
        }
        // Older gestures reached here with only the press position recorded.
        if path.is_empty() {
            path.push(from);
        }

        let Some(gesture) = cut::read_stroke(&path, CLICK_SLOP_PX) else {
            // A click is not a cut. Without this, arming the tool and clicking
            // once would take an arbitrary half of the model away. Ctrl only
            // changes which field the region is written into, so the same slop
            // applies and the word changes with it.
            //
            // It also catches a stroke that went somewhere and enclosed
            // nothing -- out and straight back -- which is not a click and is
            // just as much not a region. Hulling that into a sliver thinner
            // than the lattice would remove material in some places and not
            // others depending on where the voxels fell, while reporting
            // success.
            let what = if self.control { "mask" } else { "cut" };
            self.status = format!("{what} cancelled: drag a shape across the model");
            self.refresh_overlay();
            return;
        };

        let Some(planes) = self.cutter_from_hull(&gesture.hull) else {
            // All or nothing: dropping one side plane of a polygon does not
            // make a smaller cutter, it opens the region into an unbounded
            // wedge that carries on across the whole model.
            self.status = "cut cancelled: that shape has no direction".to_string();
            self.refresh_overlay();
            return;
        };

        // **The width guard is in VOXELS and not in pixels, and that is the
        // whole point of it.** A long thin loop clears any plausible area
        // threshold in pixels while enclosing a lens a fraction of a voxel
        // across, and whether any voxel centre falls inside such a lens depends
        // on where the lattice happens to sit. The slot then appears in some
        // places and not others, or nowhere at all -- after bricks were
        // classified, promoted, written and counted, so the status line reports
        // a successful cut over a model that did not visibly change.
        //
        // Measured at the focus depth, which is where the preview is drawn and
        // therefore where the user believes the shape is.
        if gesture.hull.len() >= 3 {
            let across = self.hull_width_in_voxels(&gesture.hull);
            if across < MIN_CUT_WIDTH_VOXELS {
                self.status = "cut cancelled: that loop is thinner than the voxel grid".to_string();
                self.refresh_overlay();
                return;
            }
        }

        // **Alt keeps both sides, and keys them.** The same drag as a cut,
        // read at the release like ctrl and shift: instead of one side going,
        // the body becomes two that register on each other. Before the depth
        // cap and the mirror, because a parting is one whole plane through one
        // body and neither applies to it.
        if self.parting_cut() {
            self.part_along(&planes, gesture.shape);
            // One shot per arming, as the cut and the mask are.
            self.tool = Tool::Sculpt;
            self.refresh_overlay();
            return;
        }

        // **Bounded by default, and shift is what asks for the old behaviour.**
        // The default is the one that answers the request -- circle the spur
        // and the spur goes -- and the unbounded cut is a modifier away rather
        // than the other way round, because a cut that takes more than it was
        // asked for is the complaint every surveyed tool has and a cut that
        // takes less is one more drag.
        //
        // Read at the RELEASE, like ctrl, and previewed while it is held, so
        // the verb is visible rather than inferred from a key state nobody can
        // see. Silent mode flips at release are named in the research as the
        // top pitfall of this whole family of tools.
        let mut depth_note: Option<&'static str> = None;
        let mut planes = planes;
        let mut depth_mm = None;
        if !self.shift
            && let Some((cap, thickness)) = self.depth_cap(&gesture.hull, &mut depth_note)
        {
            depth_mm = Some(thickness);
            planes.push(cap);
        }

        // **Ctrl turns the cut into a selection**, which is increment 25c's
        // "no new gesture": the same drag, the same region, the same brick
        // classification, writing protection instead of removing material. It
        // is checked here at the RELEASE rather than at the press, because that
        // is where the region exists and because a user who realises mid-drag
        // that they meant to select can still say so.
        //
        // EVERY shape routes here, not just the straight drag. `planes` is
        // whatever `cutter_from_hull` built -- the whole convex region and its
        // depth cap -- and `mask_cutter` reads the shape only to choose a word
        // for the status line. (This paragraph said the opposite for as long as
        // the shaped cut has existed, which is how its status line came to
        // describe a lasso and name a button that only a half-space user could
        // ever have found.)
        // **The cut honours the mirror the way every brush does.** With X on,
        // a stroke lands on both sides, so a cut with X on takes both sides:
        // the cutter the user drew and its reflection through each enabled
        // plane, depth cap included, as ONE gesture and ONE undo entry. The
        // report that prompted this had a user cut one side of a model with X
        // on and find the other side untouched, after a session in which every
        // stroke had gone to both. The preview draws the twin for the same
        // reason: what is shown is what goes, on both sides of the plane.
        let twins = self.framed_symmetry().mirrored_cutters(&planes, MIRROR_CENTRE);
        let mut cutters: Vec<&[hani_core::ClipPlane]> = vec![&planes];
        cutters.extend(twins.iter().map(Vec::as_slice));
        let mirrored = if twins.is_empty() {
            String::new()
        } else {
            format!(", mirrored {}", self.symmetry_settings().label())
        };

        if self.control {
            self.mask_cutter(&cutters, gesture.shape);
            // One shot per arming, exactly as the cut is: the assignment below
            // carries that property and this arm must not skip it.
            self.tool = Tool::Sculpt;
            self.refresh_overlay();
            return;
        }

        // **The cut crosses every VISIBLE body**, which is what
        // `Document::clip_cutters` is for: solo narrows the set, a hidden body
        // the gesture passes over comes back bit-identical, and the whole
        // gesture is ONE undo entry of N `Change::Bricks`. Direct manipulation
        // acts on what is drawn.
        let visible = self.drawn_nodes();
        let outcome = self.doc.clip_cutters(&cutters, &visible);
        if outcome.bricks > 0 {
            let before = self.history.stats();
            if let Some(entry) = outcome.entry {
                self.history.push(entry);
            }
            self.unsaved = true;
            let where_ = match gesture.shape {
                cut::CutShape::Line => String::new(),
                cut::CutShape::Curve => " inside the curve".to_string(),
                cut::CutShape::Lasso => " inside the lasso".to_string(),
            };
            // How deep it went, which is the whole difference between this and
            // the cut that shipped before it. A shaped cut that says only how
            // many bricks it took leaves the user to discover from the other
            // side whether it stopped.
            let how_deep = match (depth_mm, depth_note) {
                (_, Some(note)) => format!(" — {note}"),
                (Some(deep), None) => format!(", {deep:.0} mm deep"),
                (None, None) if gesture.shape == cut::CutShape::Line => String::new(),
                (None, None) => " — all the way through".to_string(),
            };
            let body_count = if outcome.bodies_cut.len() > 1 {
                format!(" across {} bodies", outcome.bodies_cut.len())
            } else {
                String::new()
            };
            self.status = format!(
                "cut {} bricks{where_}{body_count}{mirrored}{how_deep}{}",
                outcome.bricks,
                self.loose_pieces_note(&outcome.bodies_cut)
            );
            // After the cut's own message on purpose: an eviction that took
            // a deleted body with it outranks a count of bricks.
            self.record_history(before);
            self.remesh_dirty();
            self.refresh_overlay();
        } else {
            // Nothing changed, so nothing is recorded -- an undo entry for a
            // no-op would be worse than none. The three ways to get here read
            // very differently to a user, so they are told apart: a mask that
            // blocked the whole cut, a line that really did pass over a body
            // and found only empty space inside its box, and a line that went
            // nowhere near anything.
            self.status = if outcome.bricks_spared_by_mask > 0 {
                // **The mask comes first, and it names the body.** Without this
                // the single most confusing thing masking can do -- a cut that
                // silently does nothing because of protection painted minutes
                // ago, possibly on a body that is not even the active one --
                // reports itself as "the cut missed the model", which sends the
                // user off to redraw a line that was never the problem.
                match outcome.bodies_spared_by_mask.as_slice() {
                    [only] => format!(
                        "the mask on {} blocked the cut",
                        self.doc
                            .meta(*only)
                            .map_or_else(|| "that body".to_string(), |meta| meta.name)
                    ),
                    many => format!("the mask blocked the cut on {} bodies", many.len()),
                }
            } else if outcome.bodies_crossed > 0 {
                format!(
                    "the cut crossed {} bodies and found nothing to remove",
                    outcome.bodies_crossed
                )
            } else {
                "the cut missed the model".to_string()
            };
        }
        // One cut per arming. A destructive tool that stays live is how a
        // stray click removes half the model. **This is the assignment that
        // carries the one-shot property, not the one in the Escape arm** --
        // moving it there instead would drop it silently while the Escape test
        // still passed.
        self.tool = Tool::Sculpt;
    }

    fn finish_stroke(&mut self) {
        let timed = self.phases.enter(phases::Phase::Finish);
        self.end_the_stroke();
        self.phases.leave(timed);
    }

    /// The end of a stroke itself. [`Hani::finish_stroke`] is this, timed.
    fn end_the_stroke(&mut self) {
        self.stroke.end();
        // Releases the locked field. One gesture is one lock, so a new drag
        // starts from the surface as it now stands rather than re-warping the
        // last one's copy.
        self.move_stroke.end();
        self.move_grab = None;
        let body = self.doc.active();
        // A decal dragged out is laid as the button comes up, and its new
        // layer goes in the same entry as its dab. See `app/decal.rs`.
        self.set_the_decal_down();
        let edit = self.doc.active_volume_mut().end_stroke();
        if let Some(entry) = self.stroke_entry(body, edit) {
            let before = self.history.stats();
            self.history.push(entry);
            self.record_history(before);
            // Inside the guard on purpose: a press and release that never
            // touched the model produces no edit, and must not raise a
            // "discard your work?" prompt on the way out.
            self.unsaved = true;
            // And in the same guard, for the same reason, the picture. This is
            // the "on stroke END, never per stamp" half of the refresh rule --
            // `remesh_dirty` deliberately skips a sculpt drag, so a five second
            // stroke queues exactly one render rather than one per pointer
            // event.
            self.thumbs.geometry_changed(body);
        }
        // What a stroke's remeshes left unmeasured. See `remesh_dirty`.
        if self.measures_stale {
            self.measure_document();
        }
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

    /// Throw the sculpt away and start again from a fresh sphere.
    ///
    /// Extracted from the old `ResetSphere` arm so the menu and the panel button
    /// share one path rather than drifting.
    fn reset_sculpt(&mut self) {
        let mut volume = Volume::new(self.doc.voxel_size());
        volume.seed_sphere(Vec3::ZERO, MODEL_RADIUS_MM);
        volume.mark_everything_dirty();
        self.doc = Document::from_volume(volume);
        // History refers to bricks of the volume that just went away, so keeping
        // it would let undo splice pieces of the discarded model into this one.
        self.history.clear();
        self.history_stats = self.history.stats();
        self.camera = OrbitCamera::framing(Vec3::ZERO, MODEL_RADIUS_MM);
        // `framing` leaves the default lattice behind, and the floor is a
        // number of voxels, so it has to be told the real one.
        self.refresh_camera_lattice();
        self.project_path = None;
        // A new sculpt starts with no pictures and no floor. See
        // `app/references.rs`.
        self.adopt_references(hani_core::reference::References::default());
        self.unsaved = false;
        self.status = String::new();
        self.publish_camera();
        self.rebuild_everything();
        self.refresh_detail_advice();
        self.refresh_overlay();
    }

    /// How long between crash-net writes.
    ///
    /// Two minutes was a compromise against the write itself, when the write
    /// was on the thread that draws: often enough to be worth having and
    /// rare enough that the hitch was not something you sculpted against.
    /// The write is off that thread now (`app/autosave.rs`) and this could
    /// be shorter. It has been left, because a write still has to find the
    /// document holding still for as long as it takes.
    const AUTOSAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(120);

    /// How long the pointer must have been still before the crash net is
    /// begun.
    ///
    /// This was here to hide a hitch: the write was on the thread that
    /// draws, measured at **1.55 GB/s**, so a 159 MB model took 100 ms and the
    /// 671 MB autosave seen from a real scan session about 430 ms -- some
    /// twenty six dropped frames, every two minutes, in the middle of
    /// sculpting. Waiting for a pause put the hitch where it was not seen.
    ///
    /// There is no hitch to hide now: the encoding is a piece a frame and the
    /// file is written by another thread. See `app/autosave.rs`. The pause is
    /// kept for a different reason: a write in pieces is given up the moment
    /// the document is wanted, so one begun while the hand is still moving is
    /// work thrown away.
    const AUTOSAVE_IDLE_GAP: std::time::Duration = std::time::Duration::from_secs(3);

    /// Where the crash net lives.
    ///
    /// `$XDG_STATE_HOME`, not the config directory: this is recoverable state
    /// rather than settings, and it must never sit next to the user's own
    /// files where it could be mistaken for one.
    pub(crate) fn default_autosave_path() -> Option<std::path::PathBuf> {
        crate::paths::state_file("autosave.hani")
    }

    /// Write the crash net if it is due.
    ///
    /// Driven from the frame tick rather than a timer, because iced's `time`
    /// module is empty under this feature set -- the defaults here include
    /// `thread-pool`, not `tokio` or `smol`, so `iced::time::every` does not
    /// exist to be called.
    ///
    /// Known limitation: `window::frames()` stops arriving when the compositor
    /// stops asking for frames, so autosave stalls on a minimised or fully
    /// occluded window. That is the acceptable direction -- the case worth
    /// protecting is a crash during active sculpting, and during active
    /// sculpting there are frames. Revisit if iced gains a timer here.
    fn maybe_autosave(&mut self) {
        // One in flight goes on a piece, whatever else is so: the checks
        // below are about beginning one. See `app/autosave.rs`.
        if self.autosave.in_flight() {
            self.advance_autosave();
            return;
        }
        // The crash net is of the document, and on the workbench what is
        // in the document's place is not it.
        if self.on_the_bench() {
            return;
        }
        if !self.unsaved
            // Never mid-stroke: the document has to hold still while it is
            // written.
            || self.stroke.is_active()
            || self.drag.is_some()
            || self.last_autosave.elapsed() < Self::AUTOSAVE_INTERVAL
            // And not while the pointer is still moving. See the constant.
            || self.last_activity.elapsed() < Self::AUTOSAVE_IDLE_GAP
        {
            return;
        }
        if self.begin_autosave() {
            self.advance_autosave();
        } else {
            // So that a write that cannot begin is not tried again on every
            // frame after it. One that begins and is given up is another
            // matter: it is tried again at the next pause.
            self.last_autosave = Instant::now();
        }
    }

    /// The session state a `.hani` file carries alongside the field.
    ///
    /// Shared by the explicit save and the autosave so the two cannot record
    /// different things.
    fn project_state(&self) -> hani_core::ProjectState {
        hani_core::ProjectState {
            view: self.current_view(),
            paints: self.doc.paints().clone(),
            // See `app/references.rs`.
            references: self.references.clone(),
            // See `app/symmetry_frame.rs`.
            radial: hani_core::Radial::default(),
        }
    }

    /// What a `.hani` file stores beside the sculpt, so a reopen comes back to
    /// the same working setup.
    fn current_view(&self) -> hani_core::View {
        hani_core::View {
            camera_target: self.camera.target,
            camera_distance: self.camera.distance,
            camera_yaw: self.camera.yaw,
            camera_pitch: self.camera.pitch,
            camera_roll: self.camera.roll,
            brush_radius: self.brush.radius,
            brush_strength: self.brush.strength,
            mirror: MirrorAxis::ALL.map(|axis| self.symmetry_settings().axis(axis)),
        }
    }

    /// Put the camera, brush and mirror planes back the way a view records
    /// them, clamping every number to the range the interface offers.
    ///
    /// Clamped rather than trusted for the same reason `open_project` clamps:
    /// a view can come from a file, and the radius ceiling has already moved
    /// once. A saved 12 mm brush is fine; a saved 40 mm one from some future
    /// build must not put the slider somewhere it cannot be dragged back from.
    ///
    /// The mirror planes are put back and then **swept**, for the same reason
    /// choosing a body sweeps them: a file can name a mirror the body it was
    /// saved with does not straddle -- it was saved from a build without the
    /// refusal, or with a different body active -- and a restored mirror
    /// carves twins into empty space exactly as an enabled one does. The sweep
    /// costs one measurement of the field, at a file open or a key press,
    /// which is a user action; see
    /// [`Hani::refuse_mirrors_the_body_does_not_straddle`].
    fn apply_view(&mut self, view: &hani_core::View) {
        // **The near and far planes come from the body being restored, never
        // from the default sphere, and getting that wrong poisons the file it
        // is later saved to.**
        //
        // This used to spread `framing(Vec3::ZERO, MODEL_RADIUS_MM)` -- the
        // 30 mm starting ball -- which set `near` to 0.03 and therefore pinned
        // `zoom_by`'s floor at `near * 10.0` = 0.3 mm whatever was actually in
        // the file. On a 133 mm model the floor should be 1.33 mm, so the first
        // scroll inwards drove the camera to 0.3 and left it there: zooming in
        // did nothing, and zooming out moved 12% of 0.3 mm, which is 0.036 mm a
        // notch. Pan and orbit both scale with `distance`, so THE SPACEMOUSE,
        // THE SCROLL WHEEL AND THE NAVIGATION CUBE ALL LOOKED DEAD AT ONCE --
        // which is the shape of the report, and the reason it read as a broken
        // file rather than a broken clamp. Saving then wrote 0.3 into the
        // container, so the damage outlived the session that caused it.
        let radius =
            self.doc.active_volume().content_radius().unwrap_or(MODEL_RADIUS_MM).max(1.0e-3);
        let centre = self
            .doc
            .active_volume()
            .world_bounds()
            .map_or(Vec3::ZERO, |(low, high)| (low + high) * 0.5);
        let framed = OrbitCamera::framing(centre, radius);

        let mut camera = OrbitCamera {
            target: view.camera_target,
            distance: view.camera_distance,
            yaw: view.camera_yaw,
            pitch: view.camera_pitch,
            roll: view.camera_roll,
            ..framed
        };
        // **The admissible range is asked of a camera carrying the document's
        // own lattice and extent, which is the pair the live camera will be
        // given the moment this restore lands.**
        //
        // `framing` sets neither: it takes a radius and leaves `voxel_size` at
        // the 0.25 mm default, so `framed.min_distance()` was 0.5 mm for every
        // document ever opened while the live floor is
        // `doc.voxel_size() * 2.0`. Measured: resample to 0.05, scroll in, the
        // camera legitimately reaches 0.1 -- and reopening threw that framing
        // away and re-framed to 158 mm, with a status line telling the user
        // their file was wrong. Every lattice but 0.25 was affected, and the
        // suite could not see it because `VOXEL_SIZE_MM` happens to equal the
        // camera default.
        //
        // The ceiling had the same shape for a second reason: it came from the
        // ACTIVE body's content radius while the live one comes from the
        // document ball, so 200 notches of scroll-out parked the camera at
        // 6906 -- legal, the live clamp put it there -- and the loader called
        // 5520 the limit and rejected it. This is the same "the loader and the
        // live camera must not disagree about the same word" the target test
        // below is built on; it simply was not applied to the distance.
        camera.set_lattice(self.doc.voxel_size(), self.document_ball().1);

        // **A file written before that fix carries the bad numbers, so the
        // restore has to be able to recover one rather than merely stop making
        // them.** Two ways a saved view can be unusable, and both are checked
        // against the body rather than against a constant:
        //
        // The target may sit outside the model's bounding sphere, in which case
        // the camera is aimed at empty space and no amount of orbiting brings
        // the model back -- orbiting turns about the target, so the model stays
        // exactly as far off screen as it was. The real file that prompted this
        // had a target 435 mm from a model of radius 133 mm.
        //
        // And the distance may be outside what `zoom_by` will now admit, which
        // is what a floor computed from the wrong radius leaves behind.
        //
        // **The target test is [`Hani::document_ball`] and not the active
        // body's own sphere, so that the loader and the live camera cannot
        // disagree about the same word.** [`Hani::contain_target`] holds the
        // camera inside that ball on every mutation; if the restore accepted a
        // smaller one, a target the live camera reaches legitimately -- panned
        // across a two body document, say -- would be refused on the way back
        // in and the user's framing thrown away for no reason. The union spans
        // the gaps between bodies, which for a camera bound is the safe
        // direction to be loose in.
        //
        // The angles are kept in both cases. They are the part of a saved view
        // that is still meaningful when the rest of it is not, and re-framing
        // without them would spin a model the user had deliberately turned.
        let (ball_centre, ball_radius) = self.document_ball();
        let lost_target =
            !camera.target.is_finite() || camera.target.distance(ball_centre) > ball_radius;
        let lost_distance = !camera.distance.is_finite()
            || camera.distance < camera.min_distance()
            || camera.distance > camera.max_distance();
        if lost_target {
            camera.target = framed.target;
        }
        if lost_distance {
            // Clamped for the same reason `frame_active` clamps: a body smaller
            // than a couple of voxels frames at a distance under its own floor.
            camera.distance = framed.distance.clamp(camera.min_distance(), camera.max_distance());
        }
        if lost_target || lost_distance {
            // Said out loud rather than fixed silently: the view really is not
            // the one the file asked for, and a user who had framed a detail
            // deliberately should be told why it moved.
            self.status = format!(
                "{} — the saved view could not see the model, so it was re-framed",
                self.status
            );
        }

        self.camera = camera;
        self.refresh_camera_lattice();
        self.brush.radius = view.brush_radius.clamp(MIN_RADIUS_MM, self.max_radius());
        self.brush.strength = view.brush_strength.clamp(MIN_STRENGTH, MAX_STRENGTH);
        // The planes are each body's own and kept with it; a file from
        // before kept them here, for every body. See `app/symmetry_frame.rs`.
        self.adopt_session_mirrors(view.mirror);
        self.refuse_mirrors_the_body_does_not_straddle("turned off");
    }

    /// Whether there is a crash net to offer.
    pub(crate) fn has_autosave(&self) -> bool {
        self.autosave_file.as_ref().is_some_and(|path| path.is_file())
    }

    /// Move an autosave this build cannot read out of the way, so the next one
    /// cannot destroy it.
    ///
    /// **The updater supports downgrade, and that is what makes this
    /// necessary.** Publishing a manifest naming an older build is how a bad
    /// release is walked back, and `apply.rs` states outright that the build
    /// reading a file can be older than the build that wrote it. Compose that
    /// with the autosave: a newer build writes a crash net in a field encoding
    /// the older one refuses, the user is told an autosave is waiting, File >
    /// Recover fails with a version message, they shrug and start sculpting --
    /// and 120 seconds later the older build overwrites the only copy of their
    /// work with an empty sphere. Nothing in the write path notices that the
    /// file it is about to replace is one it could not read.
    ///
    /// Probed with [`read_outline`], which reads a header and a node table
    /// rather than a document, so the check costs a few hundred bytes on a file
    /// that may be a gigabyte. Any error moves it aside, not only a version
    /// one: a truncated or corrupt crash net is equally worth keeping, since
    /// it is the only copy of something and this application cannot say what a
    /// later build or a hex editor will make of it.
    ///
    /// Named by the first free number rather than by a timestamp, so a test can
    /// assert the name it lands on.
    fn preserve_unreadable_autosave(&mut self) -> Option<std::path::PathBuf> {
        let path = self.autosave_file.clone()?;
        let file = std::fs::File::open(&path).ok()?;
        let mut reader = std::io::BufReader::new(file);
        if hani_core::project::read_outline(&mut reader).is_ok() {
            return None;
        }
        drop(reader);

        // A bounded search: a directory already holding this many refused crash
        // nets is a bigger problem than the naming, and spinning here would be
        // worse than giving up.
        for attempt in 1..100 {
            let aside = path.with_file_name(format!("autosave-unreadable-{attempt}.hani"));
            if aside.exists() {
                continue;
            }
            return match std::fs::rename(&path, &aside) {
                Ok(()) => {
                    log::warn!("kept an unreadable autosave as {}", aside.display());
                    Some(aside)
                }
                Err(error) => {
                    log::error!("could not move {} aside: {error}", path.display());
                    None
                }
            };
        }
        None
    }

    /// Load the crash net, and deliberately do not adopt its path.
    ///
    /// `open_project` would leave `project_path` pointing at the autosave file,
    /// and the next plain Save would then write the user's work back into the
    /// crash net -- a file under `$XDG_STATE_HOME` that they never chose and
    /// that the next successful save deletes. So the recovered document is
    /// deliberately homeless and unsaved: Save behaves as Save As, which is
    /// what makes them name it somewhere real.
    fn recover_autosave(&mut self) {
        let Some(path) = self.autosave_file.clone() else {
            return;
        };
        self.open_project(&path);
        if self.status.contains("could not") {
            return;
        }
        self.project_path = None;
        self.unsaved = true;
        // `open_project` records whatever it opened. The autosave is a crash
        // net, not a document, and has no business in the recent list.
        self.recent.forget(&path);
        self.status = "recovered the autosave — save it somewhere".to_string();
    }

    fn open_project(&mut self, path: &std::path::Path) {
        // A file picked in a browser is held by name. See `files.rs`.
        let mut reader = match crate::files::open(path) {
            Ok(file) => file,
            Err(error) => {
                self.status = format!("could not open {}: {error}", path.display());
                // A file that has been moved or deleted since it was last used
                // should stop being offered.
                self.recent.forget(path);
                return;
            }
        };
        let (doc, state) = match hani_core::project::read(&mut reader) {
            Ok(loaded) => loaded,
            Err(error) => {
                // The message says what was expected as well as what was found,
                // because "a file from a different build" and "a partial
                // download" look identical without it.
                self.status = format!("could not read {}: {error}", path.display());
                return;
            }
        };

        self.doc = doc;
        // The mask card is keyed on `(body, mask revision)`, and a mask read
        // from a file has the revision the READER left on it -- which counts
        // the bricks it inserted. Two projects whose active body shares an id
        // and a mask brick count would then hit each other's cache and the
        // card would stand there naming the wrong percentage. Nothing of the
        // old document survives an open, this included.
        self.mask_card = None;
        // Before `apply_view`, so that a mirror plane the loaded body does not
        // straddle wins the status line: which file was opened is visible in
        // the title and the recent list, and a mirror plane going off is not
        // visible anywhere else. Same ordering, and same reason, as
        // `select_body`.
        self.status = format!("opened {}", path.display());
        self.apply_view(&state.view);
        // A file from before each body had its own radial copies. See
        // `app/symmetry_frame.rs`.
        self.adopt_session_radial(state.radial);
        self.doc.replace_paints(state.paints);
        // The pictures and the floor. See `app/references.rs`.
        self.adopt_references(state.references);
        // Painted by filament slot, from before slot painting was taken
        // out: its slots laid into its colour paint. See
        // `hani_core::paint::old_slots`.
        self.lay_old_slots();

        // History belongs to the model that just went away.
        self.history.clear();
        self.history_stats = self.history.stats();
        self.project_path = Some(path.to_path_buf());
        self.unsaved = false;
        self.recent.record(path);
        self.publish_camera();
        self.rebuild_everything();
        self.refresh_detail_advice();
        self.refresh_overlay();
    }

    /// Swap an imported model in, modelled on `open_project`.
    ///
    /// Three things here are individually silent when wrong.
    ///
    /// `project_path` is cleared, where `open_project` sets it. An import must
    /// not adopt the mesh's path, or the next plain Save would write a `.hani`
    /// container straight over the user's `.stl` with no dialog and no warning.
    /// This is the most damaging mistake available in this function.
    ///
    /// `mark_everything_dirty` is done by `voxelise` itself, exactly as
    /// `project::read` does it, which is why neither this nor `open_project`
    /// appears to need it.
    ///
    /// The camera is framed on `MODEL_RADIUS_MM` like every other call site,
    /// not on the model's own measured radius: the bound rounds out to whole
    /// bricks, so for a centred model it over-reports by up to about 1.7 times
    /// plus padding and the camera sits visibly too far back.
    fn adopt_import(&mut self, imported: crate::message::Imported) {
        self.doc = Document::from_volume(imported.volume);
        self.camera = OrbitCamera::framing(Vec3::ZERO, MODEL_RADIUS_MM);
        // `framing` leaves the default lattice behind, and the floor is a
        // number of voxels, so it has to be told the real one.
        self.refresh_camera_lattice();
        // **The brush is in millimetres and the model just changed size.** The
        // 3 mm default is a twentieth of the 60 mm starting ball, which is a
        // brush; against a 200 mm import it is a pin, and against a model that
        // arrived small it covered the whole thing. Neither is a radius anyone
        // chose, and both read as the controls being broken rather than as a
        // slider needing a drag. Held at the same twentieth so the brush means
        // the same thing whatever was opened.
        let longest = imported.report.longest_mm;
        if longest.is_finite() && longest > 0.0 {
            self.brush.radius = (longest / 20.0).clamp(MIN_RADIUS_MM, self.max_radius());
        }
        self.history.clear();
        self.history_stats = self.history.stats();
        self.project_path = None;
        // An import is unsaved work by definition: there is no `.hani` file
        // holding it, and quitting now would lose the whole thing.
        self.unsaved = true;
        self.status = format!(
            "imported {}: {}, {:.0} ms",
            imported
                .source
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| imported.source.display().to_string()),
            imported.report.summary(),
            imported.elapsed_ms,
        );
        log::info!("{}", self.status);
        // Nothing in an STL, OBJ or 3MF says which axis is up, and the two
        // conventions in the world disagree -- so a mesh that arrived lying
        // down is a normal outcome rather than a corrupt file. Ask; do not
        // turn it silently, because the guess can be wrong and a model turned
        // without being asked about is one the user cannot account for.
        //
        // Filtered against `Up`, so the common case of a print-ready file that
        // was already standing raises nothing.
        self.orient_prompt = imported.resting_up.filter(|up| *up != hani_core::Facing::Up);
        self.publish_camera();
        self.rebuild_everything();
        self.refresh_detail_advice();
        self.refresh_overlay();
    }

    /// Save the document, through a temporary file and a rename.
    ///
    /// **Two things here used to be wrong, and both got worse with every body
    /// added.**
    ///
    /// It opened the user's real file with `File::create`, which truncates it
    /// to zero before a byte of the new document is written. The crash net
    /// already had a temp-and-rename and the document did not, so the *net* was
    /// better protected than the thing it protects -- and at the measured
    /// 1.55 GB/s a three gigabyte document is about two seconds of the user's
    /// file existing as a truncated stub. A failed save now leaves the previous
    /// file exactly as it was.
    ///
    /// And it cleared `unsaved` and deleted the crash net **on the writer's
    /// word**. So the file is read back before either happens: its header and
    /// node table only, a few hundred bytes and no geometry, compared against
    /// the document that was meant to go into it. On a mismatch the temporary
    /// is thrown away, the previous file is untouched, `unsaved` stays set, the
    /// autosave survives, and the status says "could not" -- which is what
    /// `panel.rs` colours red.
    fn save_project(&mut self, path: &std::path::Path) {
        let state = self.project_state();

        // Beside the target rather than in a temp directory, so the rename is
        // within one filesystem and therefore atomic.
        let temporary = path.with_extension("hani.tmp");
        let file = match std::fs::File::create(&temporary) {
            Ok(file) => file,
            Err(error) => {
                self.status = format!("could not write {}: {error}", path.display());
                return;
            }
        };
        let mut writer = std::io::BufWriter::new(file);
        if let Err(error) = hani_core::project::write(&mut writer, &self.doc, &state) {
            self.status = format!("could not write {}: {error}", path.display());
            std::fs::remove_file(&temporary).ok();
            return;
        }
        // `project::write` ends with its own flush and reports a failure there
        // as an error, so nothing is left in the buffer by the time this
        // returns. The drop is to CLOSE the file before it is reopened for the
        // check below.
        drop(writer);

        if let Err(problem) = self.verify_written(&temporary) {
            self.status = format!("could not write {}: {problem}", path.display());
            std::fs::remove_file(&temporary).ok();
            return;
        }

        if let Err(error) = std::fs::rename(&temporary, path) {
            self.status = format!("could not replace {}: {error}", path.display());
            std::fs::remove_file(&temporary).ok();
            return;
        }

        self.project_path = Some(path.to_path_buf());
        // Only here. Every failure arm above leaves the flag set, or a failed
        // write would let the next quit go through silently.
        self.unsaved = false;
        self.recent.record(path);
        // The work is safely in a file the user chose, so the crash net has
        // nothing left to protect and a stale one would only offer to restore
        // something older than what is on disk.
        self.clear_autosave();
        self.status = format!("saved {}", path.display());
    }

    /// Read back what was just written, far enough to know it is this document.
    ///
    /// The header and the node table, which is a few hundred bytes whatever the
    /// sculpt weighs -- the geometry is deliberately not re-read, because doing
    /// that on the draw thread would double the cost of every save.
    ///
    /// What it catches is the class the old code could not: a write that
    /// reported success and produced a file this build will not open, or opens
    /// as a different document. That is not hypothetical -- the format grew a
    /// node table this release, and the first thing to exercise it in the wild
    /// is the autosave, unwatched, every two minutes.
    ///
    /// The check itself is pinned by
    /// `the_save_verification_notices_a_file_that_is_not_this_document`, and
    /// the *gate* in `save_project` that calls it -- a separate thing, and the
    /// one that can be refactored away without a test noticing -- by
    /// `a_save_that_cannot_read_its_own_output_back_keeps_the_previous_file`.
    /// If you change the shape of the failure here, that second test is the one
    /// to look at: it drives this through a temporary that reads back empty.
    fn verify_written(&self, path: &std::path::Path) -> Result<(), String> {
        let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
        self.check_written(&mut std::io::BufReader::new(file))
    }

    /// The check of [`Self::verify_written`], on bytes from anywhere: in a
    /// browser they are checked in memory. See `app/shelf.rs`.
    fn check_written(&self, reader: &mut impl std::io::Read) -> Result<(), String> {
        let outline =
            hani_core::project::read_outline(reader).map_err(|error| error.to_string())?;
        if outline.nodes != self.doc.node_count() || outline.bodies != self.doc.body_count() {
            return Err(format!(
                "the file came back with {} rows and {} bodies, and the document has {} and {}",
                outline.nodes,
                outline.bodies,
                self.doc.node_count(),
                self.doc.body_count()
            ));
        }
        if outline.voxel_size != self.doc.voxel_size() {
            return Err(format!(
                "the file came back at a {} mm voxel, and the document is at {} mm",
                outline.voxel_size,
                self.doc.voxel_size()
            ));
        }
        Ok(())
    }

    /// Weld every visible body and write them all out to a chosen path.
    ///
    /// Refuses to write anything that would not print. A slicer given a mesh
    /// with holes either rejects it or fills it wrong, and finding that out
    /// after a failed print is worse than being told here.
    ///
    /// **The refusal happens before `File::create`, and that ordering is the
    /// point.** `File::create` truncates, so a verdict taken after it would
    /// leave the user with a zero-byte file where their last good export was.
    /// Everything is welded and validated into memory, the verdict is taken,
    /// and only then is a file opened.
    ///
    /// **The status line names how many bodies were omitted, unconditionally
    /// -- including when the answer is none.** The eye is one bit in a
    /// forty-byte node record with no integrity check of any kind, and it is
    /// the bit that decides whether a part reaches the printer: a single
    /// flipped bit is a perfectly legal value that loads without error. The
    /// brick stream is defended by a checked distance decode and the node table
    /// is defended by nothing, so this line is the whole of that defence and it
    /// costs one format string. Making it conditional -- "only say it when
    /// something was hidden" -- would mean silence is ambiguous between
    /// "nothing was hidden" and "the count was never computed", which is
    /// exactly the reading it exists to prevent.
    ///
    /// `saved_visibility` and never `display_visibility`: a view mode must not
    /// decide what reaches a print.
    fn export(&mut self, format: ExportFormat, path: &std::path::Path) {
        let started = Instant::now();
        let mut visible = Vec::new();
        self.doc.saved_visibility(&mut visible);
        // Decimated or not, what comes back is welded, validated and judged
        // by the same verdict below. A decimated body that would not print is
        // handed back at full density by the decimator itself, and says so.
        let (mut bodies, decimated) = match self.export_detail.tolerance_mm() {
            None => (self.doc.export_bodies(&visible), None),
            Some(tolerance_mm) => {
                let spec = hani_core::export::decimate::DecimateSpec::within(tolerance_mm);
                let (bodies, reports) = self.doc.export_bodies_decimated(&visible, spec);
                let total = hani_core::export::decimate::DecimateReport::summed(&reports);
                (bodies, Some(total))
            }
        };
        let omitted = self.doc.body_count().saturating_sub(bodies.len());

        // Per body, never over the union: a document whose second body is
        // empty sums to a printable report while half the print is missing.
        if let Err(why) = hani_core::export::document_verdict(&bodies) {
            self.status = format!("not exported, {why}");
            log::error!("refusing to export: {why}");
            return;
        }

        // Paints are worked out on screen and stored nowhere, and a file has
        // to hold something: a colour a vertex for a PLY, a filament a
        // triangle for a 3MF. See `app/paints.rs`.
        let colours = self.bake_for(format, &mut bodies);

        let summary =
            hani_core::MeshReport::summed(bodies.iter().map(|(_, _, report)| *report)).summary();
        let parts: Vec<(&str, &hani_core::ExportMesh)> =
            bodies.iter().map(|(meta, mesh, _)| (meta.name.as_str(), mesh)).collect();

        let filaments = self.palette.as_filaments();
        // Read once, here, rather than inside the writer: both of these walk the
        // user's OrcaSlicer presets off disk, which is not something an export
        // writer should be doing per body.
        let placement =
            hani_core::export::threemf::Placement { plate_centre: crate::slicer::plate_centre() };
        let settings = crate::slicer::project_settings_body(&self.palette);
        let write = || -> std::io::Result<()> {
            // In a browser, a download. See `files.rs`.
            let file = crate::files::create(path)?;
            let mut file = std::io::BufWriter::new(file);
            match format {
                ExportFormat::Stl => hani_core::export::stl::write_all(&parts, &mut file)?,
                ExportFormat::Obj => hani_core::export::obj::write_all(&parts, &mut file)?,
                // The 3MF carries the user's own filament slots, sits on their
                // bed, and names the presets their slicer already has -- none of
                // which the geometry knows anything about.
                ExportFormat::ThreeMf => hani_core::export::threemf::write_project(
                    &parts,
                    &hani_core::export::threemf::Project {
                        filaments: filaments.clone(),
                        placement,
                        settings: settings.as_deref(),
                    },
                    &mut file,
                )?,
                ExportFormat::Textured => self.write_textured(&bodies, path, &mut file)?,
                ExportFormat::Glb => self.write_glb(&bodies, &mut file)?,
                ExportFormat::Ply => {
                    let coloured: Vec<hani_core::export::ply::Coloured<'_>> = bodies
                        .iter()
                        .zip(&colours)
                        .map(|((_, mesh, _), colours)| hani_core::export::ply::Coloured {
                            mesh,
                            colours,
                        })
                        .collect();
                    hani_core::export::ply::write_all(&coloured, &mut file)?;
                }
            }
            std::io::Write::flush(&mut file)
        };

        match write() {
            Ok(()) => {
                let bytes = crate::files::size(path);
                // STL has nowhere to put an object boundary, so several bodies
                // arrive in a slicer as one part. Said here rather than
                // discovered after slicing.
                let fused = matches!(format, ExportFormat::Stl) && parts.len() > 1;
                // No silent fallback. Without the user's presets the file still
                // opens, but OrcaSlicer invents a machine named after it and
                // displaces whatever printer they had selected -- which looks
                // like the application broke their slicer rather than like a
                // setting that could not be read.
                let unbound = matches!(format, ExportFormat::ThreeMf) && settings.is_none();
                // What the decimation did, in the decimator's own words, or
                // nothing at all for a full export: the line an existing
                // export writes is unchanged.
                let decimation =
                    decimated.map(|report| format!("; {}", report.summary())).unwrap_or_default();
                // And how much of it overhangs, when the overhang view is
                // on and so somebody is asking. See `app/overhang.rs`.
                let decimation = format!("{decimation}{}", self.overhang_summary(&bodies));
                self.status = format!(
                    "exported {} of {} bodies; {omitted} hidden{} -- {summary}{decimation} to {} \
                     ({:.1} MB, {:.0} ms){}",
                    parts.len(),
                    self.doc.body_count(),
                    if fused { " (STL fuses them into one part)" } else { "" },
                    path.display(),
                    bytes as f64 / (1024.0 * 1024.0),
                    started.elapsed().as_secs_f64() * 1000.0,
                    if unbound {
                        " — no printer named: pick your machine in OrcaSlicer, then export again"
                    } else {
                        ""
                    }
                );
                log::info!("{}", self.status);
            }
            Err(error) => {
                self.status = format!("could not write {}: {error}", path.display());
                log::error!("{}", self.status);
            }
        }
    }

    /// Export the sculpt to a staging file and open it in OrcaSlicer.
    ///
    /// A staging path under the state directory rather than a dialog: this is a
    /// handoff and not a save, the file is the slicer's input rather than the
    /// user's document, and asking where to put something they will not look
    /// for again is a question with no useful answer.
    ///
    /// 3MF rather than STL, because it carries units. An STL does not, so a
    /// slicer has to guess millimetres, and every slicer guessing right is a
    /// convention rather than a guarantee.
    ///
    /// Runs on the update thread, like `export` already does. The export itself
    /// is the slow half and it is the same work the Export buttons do.
    fn hand_to_slicer(&mut self) {
        let Some(slicer) = crate::slicer::find() else {
            self.status = "could not find OrcaSlicer — install it and try again".to_string();
            return;
        };
        let Some(staged) = crate::paths::state_file("handoff.3mf") else {
            self.status = "could not work out where to put the file".to_string();
            return;
        };
        if let Some(parent) = staged.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            self.status = format!("could not make {}: {error}", parent.display());
            return;
        }

        self.export(ExportFormat::ThreeMf, &staged);
        // `export` refuses a model that would not print, and says so in the
        // status. Handing the slicer a file that was never written would
        // replace that message with a less useful one.
        if !staged.is_file() || self.status.contains("not exported") {
            return;
        }

        match crate::slicer::open(&slicer, &staged) {
            Ok(()) => {
                self.status = format!("opened in {}", slicer.display());
            }
            Err(why) => self.status = format!("could not open the slicer: {why}"),
        }
    }

    /// The voxel size a request actually lands on.
    fn clamped_voxel_size(requested: f32) -> f32 {
        requested.clamp(FINEST_VOXEL_MM, COARSEST_VOXEL_MM)
    }

    /// Rebuild the volume at a different voxel size.
    fn resample(&mut self, voxel_size: f32) {
        let mut voxel_size = Self::clamped_voxel_size(voxel_size);
        if (voxel_size - self.doc.voxel_size()).abs() < 1.0e-6 {
            return;
        }

        // A step the pool cannot hold LANDS ON THE FINEST THAT FITS instead
        // of refusing. The refusal named that size and then made the user go
        // there by hand -- which they could not: the button only halves, so
        // the message pointed at a rung the interface had no way to reach.
        // Observed live at 0.062 mm with 0.043 mm reachable and no path to it.
        let mut capped = None;
        if let Some((why, finest)) = self.too_fine_for_the_pool(voxel_size) {
            let fallback = Self::clamped_voxel_size(finest);
            if fallback < self.doc.voxel_size() * 0.98 {
                capped = Some(voxel_size);
                voxel_size = fallback;
            } else {
                // Already at the fit limit: there is nowhere finer to land.
                self.status = why;
                log::warn!("{}", self.status);
                return;
            }
        }

        let started = Instant::now();
        // Every body at once, because they share the lattice: the old bricks
        // have to be cleared out of the renderer's pool, and after a resample
        // their coordinates mean something different, so they are marked in the
        // new volume to remesh to nothing.
        self.doc.resample(voxel_size);
        // Past the early return above, so resampling to the size already in use
        // stays the no-op its test asserts it is.
        self.unsaved = true;
        // History refers to bricks of a volume that no longer exists, and at a
        // different resolution, so keeping it would splice nonsense back in.
        self.history.clear();
        self.history_stats = self.history.stats();
        self.rebuild_everything();
        self.refresh_detail_advice();

        self.status = match capped {
            Some(wanted) => format!(
                "resampled to {voxel_size:.3} mm -- the finest the mesh pool holds at this size \
                 ({wanted:.3} mm did not fit), {} dense bricks, {:.0} MB, {:.0} ms",
                self.doc_stats.dense_bricks,
                self.doc_stats.resident_bytes as f64 / (1024.0 * 1024.0),
                started.elapsed().as_secs_f64() * 1000.0
            ),
            None => format!(
                "resampled to {voxel_size:.3} mm, {} dense bricks, {:.0} MB, {:.0} ms",
                self.doc_stats.dense_bricks,
                self.doc_stats.resident_bytes as f64 / (1024.0 * 1024.0),
                started.elapsed().as_secs_f64() * 1000.0
            ),
        };
        log::info!("{}", self.status);
    }

    /// Why a resample to `wanted` would not fit the mesh pool, if it would not.
    ///
    /// This is the guard the import path has had all along and this one never
    /// did, and its absence was not theoretical: on the Nightwing dragon the
    /// "finer" button resampled correctly in 296 ms and then reserved 8,525,824
    /// vertices against a pool of 8,000,000. The volume was right, the model on
    /// screen lost parts of itself, and the only thing that said so was a line
    /// on stderr. From the user's chair the button did nothing.
    ///
    /// The prediction is a square law over what is ACTUALLY on the GPU right
    /// now, not an estimate from surface area. A surface has a fixed area, so
    /// halving the voxel size quadruples the vertices on it -- and starting
    /// from a measured reservation means the allocator's own padding is already
    /// baked into the figure rather than guessed at.
    fn too_fine_for_the_pool(&self, wanted: f32) -> Option<(String, f32)> {
        if wanted >= self.doc.voxel_size() {
            return None;
        }

        // System memory first, because the GPU pool is no longer the tighter
        // of the two. Measured on the dragon: 0.0565 mm fits the pool at 48%
        // and costs 4.15 GB of RAM, so a pool-only guard would happily walk a
        // machine into swap or the OOM killer. Same square law -- a surface
        // has fixed area, so halving the voxel quadruples the shell.
        let growth = (self.doc.voxel_size() / wanted).powi(2) as f64;
        let bytes = self.doc_stats.resident_bytes as f64 * growth;
        if bytes > MAX_VOLUME_BYTES {
            let headroom = MAX_VOLUME_BYTES / self.doc_stats.resident_bytes.max(1) as f64;
            let finest = self.doc.voxel_size() / headroom.sqrt() as f32 * 1.03;
            return Some((
                format!(
                    "could not resample to {wanted:.3} mm: it needs about {:.1} GB of memory \
                     against a {:.0} GB ceiling -- {finest:.3} mm is the finest that fits",
                    bytes / (1024.0 * 1024.0 * 1024.0),
                    MAX_VOLUME_BYTES / (1024.0 * 1024.0 * 1024.0),
                ),
                finest,
            ));
        }

        let pool = self.shared.stats();
        if pool.vertices_reserved == 0 {
            return None;
        }
        // Predicted from what is LIVE, because a resample resets the pool and
        // so starts from a clean bump pointer -- the watermark before the
        // reset says nothing about the space after it. Scaling `live` is
        // therefore the honest basis, and it is only honest BECAUSE of the
        // reset; before that existed this prediction was blind to
        // fragmentation and let the pool overflow.
        let vertices = pool.vertices_reserved as f64 * growth;
        let indices = pool.indices_reserved as f64 * growth;
        if vertices <= pool.vertex_capacity as f64 && indices <= pool.index_capacity as f64 {
            return None;
        }

        // The finest size that would fit, from the same square law, with three
        // percent on top because a prediction that lands at exactly 100% of a
        // ceiling helps nobody. Returned so `resample` can GO there rather
        // than tell the user to.
        let headroom = (pool.vertex_capacity as f64 / pool.vertices_reserved as f64)
            .min(pool.index_capacity as f64 / pool.indices_reserved as f64);
        let finest = self.doc.voxel_size() / headroom.sqrt() as f32 * 1.03;
        let why = format!(
            "could not resample to {wanted:.3} mm: it needs about {:.1}M vertices against a pool \
             of {:.1}M -- {finest:.3} mm is the finest that fits at this size",
            vertices / 1.0e6,
            pool.vertex_capacity as f64 / 1.0e6,
        );
        Some((why, finest))
    }

    /// Scale the model so its longest dimension is `longest_mm`.
    ///
    /// Free and lossless: distances are stored in voxels, so this is a change
    /// to `voxel_size` and nothing else, and the bricks are bit-identical
    /// afterwards. It buys **no** detail -- the model has the same number of
    /// voxels across it as before. What it changes is what one voxel measures,
    /// which is what decides whether the detail already there is enough for a
    /// given printer. [`Hani::detail_advice`] is what tells the user that.
    fn set_working_size(&mut self, longest_mm: f32) {
        if !(longest_mm.is_finite() && longest_mm > 0.0) {
            self.status = "could not resize: that is not a length".into();
            return;
        }
        let Some((lo, hi)) = self.doc.active_volume().surface_bounds() else {
            self.status = "could not resize: there is no model".into();
            return;
        };
        let current = (hi - lo).max_element();
        if current <= 0.0 {
            self.status = "could not resize: the model has no size".into();
            return;
        }

        let factor = longest_mm / current;
        // The voxel size travels with the model, so the range that bounds it
        // bounds this too -- otherwise a resize could land somewhere the finer
        // and coarser buttons can never get back from.
        let wanted_voxel = self.doc.voxel_size() * factor;
        let clamped = Self::clamped_voxel_size(wanted_voxel);
        if (clamped - wanted_voxel).abs() > 1.0e-9 {
            self.status = format!(
                "could not resize to {longest_mm:.1} mm: that would put the voxel at {:.4} mm, \
                 outside the {FINEST_VOXEL_MM:.2}-{COARSEST_VOXEL_MM:.2} mm range",
                wanted_voxel,
            );
            return;
        }

        let previous_radius = self.model_radius;
        // Every body, because they share the lattice: scaling one alone would
        // hand it a lattice its siblings do not have.
        self.doc.rescale(factor);
        // The brush is in millimetres, so without this a resize leaves it the
        // wrong size relative to the model it is about to be used on.
        self.brush.radius = (self.brush.radius * factor).clamp(MIN_RADIUS_MM, self.max_radius());
        self.unsaved = true;
        // Every brick's world position moved, so everything has to be redrawn
        // even though not one voxel changed.
        self.doc.mark_everything_dirty();
        self.rebuild_everything();
        let _ = previous_radius;
        self.camera = OrbitCamera::framing(Vec3::ZERO, self.model_radius.max(1.0e-3));
        // `framing` leaves the default lattice behind, and the floor is a
        // number of voxels, so it has to be told the real one.
        self.refresh_camera_lattice();
        self.publish_camera();
        self.refresh_overlay();
        self.refresh_detail_advice();
        self.status = format!("working size {longest_mm:.1} mm, {}", self.detail_advice);
        log::info!("{}", self.status);
    }

    /// Recompute the cached readout. See the field for why it is cached.
    fn refresh_detail_advice(&mut self) {
        self.detail_advice = self.measure_detail_advice();
    }

    /// What the current resolution actually means for a print, in one line.
    ///
    /// Detail is decided by how many voxels lie across the model, which does
    /// not change when it is scaled. What a printer cares about is what one
    /// voxel MEASURES. Saying both, side by side, is the whole point: it turns
    /// "is 0.25 mm fine enough?" from a guess into arithmetic.
    ///
    /// The resin figure is the comparison worth drawing because it is the
    /// demanding one -- a consumer resin printer resolves around 0.03 mm in XY,
    /// where a filament nozzle lays down 0.4 mm lines and cannot use anything
    /// like this much.
    fn measure_detail_advice(&self) -> String {
        let Some((lo, hi)) = self.doc.active_volume().surface_bounds() else {
            return "no model".into();
        };
        let voxel_size = self.doc.voxel_size();
        let longest = (hi - lo).max_element();
        let across = (longest / voxel_size).round() as i64;
        let verdict = if voxel_size <= RESIN_XY_MM {
            "at or below what a resin printer resolves"
        } else if voxel_size <= FDM_LINE_MM {
            "finer than a filament nozzle, coarser than resin"
        } else {
            "coarse: a filament nozzle would not see the difference"
        };
        format!(
            "{longest:.1} mm across, voxel {voxel_size:.3} mm, {across} voxels wide -- {verdict}"
        )
    }

    /// What one puck button press does.
    ///
    /// Split out of [`Hani::drive_from_spacemouse`] so a test can press a
    /// button without a `/dev/input` node: the press queue is filled by a
    /// reader thread, and the two camera arms below are exactly the kind of
    /// code that goes wrong unobserved because nothing in CI can reach it.
    fn run_puck_button(&mut self, action: ButtonAction) {
        match action {
            ButtonAction::None => {}
            ButtonAction::Undo => self.undo(),
            ButtonAction::Redo => self.redo(),
            // Recentre and refit without losing which way the model is being
            // looked at, which is what makes this useful mid sculpt.
            //
            // **Through `frame_active`, not a fresh `framing`.** Assigning one
            // resets `voxel_size` to the 0.25 mm default and `content_radius`
            // to the hardcoded 30 mm starting ball, and both are now
            // load-bearing: the first is the zoom floor and the second is both
            // the far plane and the length of every pick ray. Measured on a
            // document resampled to 0.05, the floor went from 0.1 to 0.5 -- a
            // puck button silently making the camera five times coarser than
            // the lattice deserves, which is the precise complaint this work
            // exists to fix. On a large model the same assignment collapses
            // `far()` to well inside the geometry, so the far half of it stops
            // being drawn and stops being pickable at once. `frame_active`
            // frames the body that is actually there and refreshes the lattice
            // from the document, which is the whole job.
            ButtonAction::FrameModel => self.frame_active(),
            // Everything back to the start, roll included. This is the way out
            // of a view that has been twisted somewhere confusing.
            ButtonAction::ResetView => {
                let start = OrbitCamera::default();
                self.camera.yaw = start.yaw;
                self.camera.pitch = start.pitch;
                self.camera.roll = start.roll;
                // The angles are the reset; the framing is `frame_active`'s,
                // for the reason above.
                self.frame_active();
                self.status = "reset the view".to_string();
            }
            ButtonAction::ToggleSymmetry => {
                // Through the same gate the strip uses, and not a bare
                // `toggled`: see `toggle_mirror`.
                self.toggle_mirror(MirrorAxis::X);
            }
        }
    }

    /// Apply one frame of puck motion, and whatever its buttons asked for.
    fn drive_from_spacemouse(&mut self, elapsed_ms: f32) {
        for action in self.spacemouse.take_presses() {
            self.run_puck_button(action);
        }

        let motion = self.spacemouse.motion();
        let moved = self.spacemouse.config.apply(
            &motion,
            elapsed_ms,
            &mut self.camera,
            self.viewport_size.y,
        );
        if moved {
            self.publish_camera();
        }
    }

    fn configure_spacemouse(&mut self, setting: SpaceMouseSetting) {
        let config = &mut self.spacemouse.config;
        // Whether this change should reach the disk now. A slider mid drag
        // should not: it sends a message per step, and `Save` follows on
        // release.
        let mut persist = true;
        match setting {
            SpaceMouseSetting::Mode(mode) => config.mode = mode,
            SpaceMouseSetting::Deadzone(value) => {
                config.deadzone = value;
                persist = false;
            }
            SpaceMouseSetting::PanSens(value) => {
                config.pan_sens = value;
                persist = false;
            }
            SpaceMouseSetting::ZoomSens(value) => {
                config.zoom_sens = value;
                persist = false;
            }
            SpaceMouseSetting::StaleMs(value) => {
                config.stale_ms = value.clamp(20.0, 1000.0) as u64;
                persist = false;
            }
            SpaceMouseSetting::OrbitSens(value) => {
                config.orbit_sens = value;
                persist = false;
            }
            SpaceMouseSetting::Binding(action, source) => {
                let invert = config.binding(action).invert;
                config.set_binding(action, AxisBinding { source, invert });
            }
            SpaceMouseSetting::Invert(action, invert) => {
                let source = config.binding(action).source;
                config.set_binding(action, AxisBinding { source, invert });
            }
            SpaceMouseSetting::Button(index, action) => {
                if let Some(slot) = config.buttons.get_mut(index) {
                    *slot = action;
                }
            }
            SpaceMouseSetting::InvertAll => config.invert_all(),
            SpaceMouseSetting::Reset => *config = SpaceMouseConfig::default(),
            SpaceMouseSetting::Save => {}
        }
        if persist {
            self.spacemouse.config.save();
        }
    }

    /// Move the number a sizing gesture is holding, from how far the pointer
    /// has travelled since it began.
    ///
    /// Horizontal only, and absolute against where the drag started, so out and
    /// back returns to exactly the original value.
    fn apply_sizing(&mut self, to: Vec2) {
        let Some(sizing) = self.sizing else {
            return;
        };
        let travel = to.x - sizing.from_pixel.x;
        match sizing.what {
            // Multiplicative, matching the [ and ] keys: the radius spans fifty
            // to one, so a fixed amount per pixel would crawl at one end and
            // jump at the other.
            SizingTarget::Radius => {
                let factor = (travel * RADIUS_PER_PIXEL).exp();
                self.brush.radius =
                    (sizing.original * factor).clamp(MIN_RADIUS_MM, self.max_radius());
            }
            // Additive: strength is already a small linear range.
            SizingTarget::Strength => {
                self.brush.strength = (sizing.original + travel * STRENGTH_PER_PIXEL)
                    .clamp(MIN_STRENGTH, self.max_strength());
            }
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

    /// Tell the renderer how to draw the mask: the active body's polarity, the
    /// bodies that disagree with it, and the tint the view preferences add up
    /// to.
    ///
    /// **Here, on the every-message path, for the reason the visibility pass
    /// itself is here.** The set of things that change any of the three is
    /// bigger than it looks -- the toggle, the slider, the held peek key,
    /// choosing another body, an undo that puts a polarity back -- and a list of
    /// them is a list that goes silently out of date. What it would go out of
    /// date INTO is a mask drawn with the wrong polarity, which shows protected
    /// material as free. Two locks, two word writes and a walk of at most
    /// `MAX_BODIES` rows asking each one a bool, so the frame tick can afford it
    /// unconditionally; it allocates nothing, which
    /// [`Hani::publish_visibility`]'s contract requires of everything in it.
    ///
    /// # One word cannot answer for a document, so it does not have to
    ///
    /// A mask is per BODY and `Uniforms::mask_inverted` is one word for the
    /// whole draw. The polarity published here is the ACTIVE body's, and every
    /// body that disagrees with it is named in a set the pool consults per
    /// bucket -- see [`hani_gpu::MaskPolarity`]. Increment 24 shipped Invert
    /// and Mask All without that set, and its review found what that costs: a
    /// Mask All on a body that is not the active one drew that body's stored
    /// zeros through the active body's polarity, so a fully protected body sat
    /// on screen with no tint on it at all.
    ///
    /// The set is relative to the word rather than absolute so that a document
    /// where every body agrees -- which is every document that has not used the
    /// verbs -- publishes an empty set and costs the draw nothing.
    fn publish_mask_view(&mut self) {
        let inverted =
            self.doc.volume(self.doc.active()).is_some_and(|volume| volume.mask().inverted());
        self.opposite_polarity.clear();
        for node in self.doc.nodes() {
            if let Some(volume) = node.volume()
                && volume.mask().inverted() != inverted
            {
                self.opposite_polarity.push(node.id);
            }
        }
        self.shared.set_opposite_polarity(&self.opposite_polarity);
        // The peek key overrides the toggle rather than being overridden by it:
        // it is what a user who switched the tint off and forgot reaches for.
        let shown = self.show_mask || self.mask_peek;
        self.shared.set_mask_view(crate::viewport::MaskView {
            inverted,
            tint: if shown { self.mask_tint } else { 0.0 },
        });
        // Published beside the mask's view for the same reason and at the same
        // cost: a fixed array and no allocation, on the frame path.
        self.shared.set_paint_view(crate::viewport::PaintView {
            shown: self.show_paint,
            flat: self.flat_paint,
            palette: self.palette.shader_palette(),
            paints: hani_gpu::PaintBlock::from(self.doc.paints()),
        });
        // The gain is what makes the setting a size in millimetres: a feature
        // `w` voxels across reads a curvature of `4 / w`, so `w / 4` brings a
        // feature of exactly that size to a full crease, and one twice as
        // broad to half of one. The document's voxel size is in the ratio, so
        // the same model at a finer voxel draws the same creases.
        let overhang = self.overhang_view();
        self.shared.set_cavity_view(crate::viewport::CavityView {
            strength: if self.show_cavity { CAVITY_STRENGTH } else { 0.0 },
            gain: CAVITY_FEATURE_MM / self.doc.voxel_size() / 4.0,
            overhang,
        });
    }

    /// Rebuild the standing mask card, cheaply enough to do it every message.
    ///
    /// **This runs on the frame tick**, so what it costs when nothing has
    /// changed is what matters, and what it must cost is nothing at all: not a
    /// `mask_fill`, and not a `format!` either -- `publish_visibility`'s own
    /// contract two functions up is that the frame path allocates NOTHING, and
    /// this is at the bottom of it. So everything above the cache gate is a walk
    /// of at most `MAX_BODIES` rows asking each body a map-empty question, both
    /// counts stay a `usize` until they are known to be new, and every `String`
    /// in here is built below the gate.
    ///
    /// Three gates, in cheapest-first order: a document with no protection
    /// anywhere leaves with `None`; a document whose active body, revision and
    /// two counts are all unchanged leaves without touching the card; and only
    /// a genuine change pays for [`hani_core::Volume::mask_fill`].
    ///
    /// # Every masked body is counted somewhere, and that is the point
    ///
    /// The card is what makes "a mask is active and nothing on screen says so"
    /// unreachable, and that has to be a DOCUMENT-wide property rather than an
    /// active-body one. Increment 24's review found it was not: only the active
    /// body and the masked bodies that were HIDDEN were counted, so a body that
    /// was masked, visible and not active fell through both. Mask all on a
    /// cube, then click another body, and the card vanished while a fully
    /// protected cube sat on screen; the first the user heard of it was a clip
    /// plane sparing the whole thing.
    fn refresh_mask_card(&mut self) {
        let active = self.doc.active();
        let active_masked = self.doc.volume(active).is_some_and(|volume| !volume.mask().is_free());

        // Masked bodies that are not the one the headline is about, split by
        // whether the user can see them. The active body is excluded from both:
        // the counts mean "as well as what this card already says", so a hidden
        // active body counting itself would be a card announcing its own
        // subject twice.
        let mut off_screen = 0usize;
        let mut elsewhere = 0usize;
        for (node, shown) in self.doc.nodes().iter().zip(&self.shown) {
            if !node.is_body() || node.id == active {
                continue;
            }
            if node.volume().is_some_and(|volume| !volume.mask().is_free()) {
                if *shown {
                    elsewhere += 1;
                } else {
                    off_screen += 1;
                }
            }
        }

        if !active_masked && off_screen == 0 && elsewhere == 0 {
            self.mask_card = None;
            return;
        }

        // Zero is the revision of a card about a body carrying no mask of its
        // own, and no live mask can collide with it: `revision` counts writes,
        // so a mask that is not free has had at least one.
        let revision =
            if active_masked { self.doc.active_volume().mask().revision() } else { NO_MASK_HERE };

        if let Some(card) = &mut self.mask_card
            && card.body == active
            && card.revision == revision
        {
            if card.off_screen_count != off_screen || card.elsewhere_count != elsewhere {
                card.rewrite_the_other_counts(off_screen, elsewhere);
            }
            return;
        }

        if !active_masked {
            // Nothing on the active body, but protection is out there on
            // something else. No percentage, because there is no one body for
            // it to be about.
            let (headline, hidden) = MaskCard::about_other_bodies(off_screen, elsewhere);
            self.mask_card = Some(MaskCard {
                body: active,
                revision,
                off_screen_count: off_screen,
                elsewhere_count: elsewhere,
                headline,
                off_screen: hidden,
                elsewhere: String::new(),
            });
            return;
        }

        let name = self.doc.node(active).map_or("this body", |node| node.name.as_str()).to_string();
        let percent = (self.doc.active_volume().mask_fill() * 100.0).round() as u32;
        self.mask_card = Some(MaskCard {
            body: active,
            revision,
            off_screen_count: off_screen,
            elsewhere_count: elsewhere,
            // Never "0%": a mask that exists but rounds to nothing still blocks
            // a cut and still weakens a stroke, and a card reading zero is a
            // card saying "ignore me".
            headline: format!("MASK — {name}, {}%", percent.max(1)),
            off_screen: MaskCard::hidden_count_line(off_screen),
            elsewhere: MaskCard::elsewhere_count_line(elsewhere),
        });
    }

    // ------------------------------------------------- the whole-mask verbs

    /// Throw the active body's mask away.
    ///
    /// **The map is MOVED into the history entry and never copied**, which is
    /// the [`Change::NodeRemoved`] trick: a gigabyte of protection is cleared
    /// with no allocation, peak memory does not rise, and `resident_bytes`
    /// falls back to what it was before the mask existed. The bytes go against
    /// the reclaim allowance rather than the 256 MB stroke budget, so a Clear
    /// does not evict the strokes behind it.
    fn clear_mask(&mut self) {
        let body = self.doc.active();
        if self.doc.active_volume().mask().is_free() {
            self.status = format!("{} carries no mask", self.active_body_name());
            return;
        }
        let volume = self.doc.active_volume_mut();
        let cleared = volume.mask().cleared(false);
        let previous = volume.replace_mask(cleared);
        self.commit_whole_mask(body, previous);
        self.status = format!("cleared the mask on {}", self.active_body_name());
    }

    /// Flip which side of the mask is protected.
    ///
    /// One bool, whatever the mask holds: no walk, no allocation, no brick
    /// marked for a remesh. Without that, ctrl+I on a lightly masked
    /// 45,567-brick model would allocate 1.04 GiB from one keystroke.
    fn invert_mask(&mut self) {
        let body = self.doc.active();
        let edit = self.doc.active_volume_mut().flip_mask_polarity();
        let before = self.history.stats();
        self.history.push(Entry::stroke(body, edit));
        self.record_history(before);
        self.unsaved = true;
        // The picture follows from `publish_mask_view`, which `update` runs
        // after every message: the polarity is a uniform and not a vertex
        // attribute, so there is deliberately nothing to remesh. The thumbnail
        // is a different matter -- see `geometry_changed`'s caller list.
        self.thumbs.geometry_changed(body);
        self.status = format!("inverted the mask on {}", self.active_body_name());
    }

    /// Protect the whole body: clear, then invert, as ONE change.
    ///
    /// They are one change and not two because the polarity rides inside the
    /// field, so `cleared(true)` IS clear-then-invert -- and undoing it puts
    /// both halves back together, which is the only state either half means
    /// anything in.
    fn mask_all(&mut self) {
        let body = self.doc.active();
        if self.doc.active_volume().mask().protects_everything() {
            self.status = format!("{} is already fully masked", self.active_body_name());
            return;
        }
        let volume = self.doc.active_volume_mut();
        let all = volume.mask().cleared(true);
        let previous = volume.replace_mask(all);
        self.commit_whole_mask(body, previous);
        self.status = format!("masked all of {}", self.active_body_name());
    }

    /// Push the entry a whole-mask change produced and put the picture back in
    /// step with it.
    fn commit_whole_mask(&mut self, body: NodeId, previous: MaskField) {
        let before = self.history.stats();
        self.history.push(Entry::new(vec![Change::WholeMask { body, mask: Box::new(previous) }]));
        self.record_history(before);
        self.unsaved = true;
        self.thumbs.geometry_changed(body);
        self.remesh_dirty();
    }

    /// The amount slider moved.
    ///
    /// **Absolute.** The first non-zero amount of a gesture takes the mask off
    /// the body and keeps it as a snapshot; every amount after that is applied
    /// to that same snapshot, so the result depends on where the slider IS and
    /// never on how it got there. Dragging back to zero moves the snapshot back
    /// onto the body bit for bit and ends the gesture with no entry at all,
    /// which is why "blur at 0.0 is a no-op" is literally true rather than
    /// approximately so.
    ///
    /// # What one step of the slider costs, said plainly
    ///
    /// A whole-body filter and a whole-body remesh, per step. The remesh is the
    /// larger half and the plan already names it: 71 ms on the reference dragon
    /// and roughly 475 ms at the 45,567 bricks the pool is sized for. The
    /// step is 0.05, so a full sweep is at most twenty of those. Two things
    /// keep it usable rather than one: the uniform fast path in
    /// [`hani_core::MaskField::filtered`] walks only the shell where
    /// protection actually changes, and a slider emits nothing when the snapped
    /// value has not moved. **If this becomes the complaint, the answer is the
    /// two-halves `iced::Task` increment 25a already specifies for the
    /// generators, not a coarser step.**
    fn mask_amount_changed(&mut self, amount: f32) {
        let amount = amount.clamp(0.0, 1.0);
        self.mask_amount = amount;
        let body = self.doc.active();

        // A gesture belongs to one body and one filter. Either changing under
        // it ends it where it stands rather than re-applying one body's
        // snapshot onto another's mask.
        if self
            .mask_grab
            .as_ref()
            .is_some_and(|grab| grab.body != body || grab.filter != self.mask_filter)
        {
            self.commit_mask_filter();
        }

        if amount <= 0.0 {
            // Put the snapshot back, exactly as it was, and push nothing.
            if let Some(grab) = self.mask_grab.take()
                && let Some(volume) = self.doc.volume_mut(grab.body)
            {
                volume.replace_mask(grab.snapshot);
                self.remesh_dirty();
            }
            return;
        }

        let filter = self.mask_filter;
        if self.mask_grab.is_none() {
            if let Some(why) = self.refuse_the_filter(filter) {
                self.status = why;
                log::warn!("{}", self.status);
                self.mask_amount = 0.0;
                return;
            }
            let snapshot = self.doc.active_volume_mut().take_mask();
            self.mask_grab = Some(MaskGrab { body, filter, snapshot });
        } else {
            // Drop what the last change of this gesture left behind BEFORE the
            // next one is built, so the peak stays at two masks and not three.
            drop(self.doc.active_volume_mut().take_mask());
        }

        let Some(grab) = &self.mask_grab else {
            return;
        };
        let next = grab.snapshot.filtered(filter, amount);
        self.doc.active_volume_mut().replace_mask(next);
        self.remesh_dirty();
    }

    /// The amount slider was let go: one entry for the whole drag.
    ///
    /// The slider goes back to zero because the filter is absolute and the
    /// amount is a property of the gesture rather than of the document -- see
    /// the field's own documentation.
    fn mask_amount_released(&mut self) {
        self.commit_mask_filter();
        self.mask_amount = 0.0;
    }

    /// Commit a live filter gesture, if there is one.
    fn commit_mask_filter(&mut self) {
        let Some(grab) = self.mask_grab.take() else {
            return;
        };
        let verb = grab.filter.done();
        let body = grab.body;
        let name = self.body_name(body);
        self.commit_whole_mask(body, grab.snapshot);
        self.status = format!("{verb} the mask on {name}");
    }

    /// Commit a filter gesture whose slider is no longer there to release it.
    ///
    /// **On the frame tick, and two `Option` comparisons.** The amount slider
    /// lives in the right-click tool menu, and the menu can go away mid-drag --
    /// Escape closes it, and Escape is also how a user dismisses anything. The
    /// active body can change under it too. Either way the `MaskAmountReleased`
    /// that would have committed the drag is never coming, and what is left is
    /// a filtered mask with no history entry and no `unsaved`: real work that
    /// ctrl+Z cannot reach and that quitting would not even prompt about.
    ///
    /// Here rather than in each of the arms that closes a menu, because a list
    /// of those is a list that goes out of date silently -- the same argument
    /// `publish_visibility` makes about the eye.
    fn settle_an_orphaned_mask_grab(&mut self) {
        let orphaned = self
            .mask_grab
            .as_ref()
            .is_some_and(|grab| self.menu.is_none() || grab.body != self.doc.active());
        if orphaned {
            self.commit_mask_filter();
            self.mask_amount = 0.0;
        }
    }

    /// Why a global filter would not fit under the memory ceiling.
    ///
    /// **Not optional, and not a tidy-up.** A filter needs the old mask while
    /// it writes the new one, and the history entry then holds the old one, so
    /// the peak is `field + 2 x mask`. A fully painted mask is 25% of its field
    /// -- 32,768 bytes a brick against 131,072 -- which makes that peak 1.5
    /// times the field: on the measured 0.0565 mm dragon, 6.22 GiB against a
    /// 6 GiB ceiling, from one button press. `History::trim` keeps a single
    /// over-budget entry by design, so the undo budget cannot save it either.
    ///
    /// **Clear, Invert and Mask All are deliberately NOT gated**, and that is
    /// the same paragraph of the plan read to its end rather than a narrowing
    /// of it: all three are O(1) in memory, none of them builds a second mask,
    /// and refusing the one verb that GIVES the memory back would trap a user
    /// with a mask they cannot remove at exactly the moment they need to.
    fn refuse_the_filter(&self, filter: MaskFilter) -> Option<String> {
        let mask = self.doc.active_volume().mask();
        let (why, _coarser) =
            mask.would_fit(mask.bytes(), self.doc_stats.resident_bytes, self.doc.voxel_size())?;
        Some(format!("could not {} the mask on {}: {why}", filter.verb(), self.active_body_name()))
    }

    // ------------------------------------------------- the generated masks

    /// The recipe a generator button means, with the sliders as they stand.
    fn recipe_for(&self, which: MaskGenerator) -> MaskRecipe {
        match which {
            MaskGenerator::Cavity => MaskRecipe::Cavity { feature_mm: self.mask_feature_mm },
            MaskGenerator::Smoothness => {
                MaskRecipe::Smoothness { feature_mm: self.mask_feature_mm }
            }
            MaskGenerator::Thickness => {
                MaskRecipe::Thickness { voxels: self.mask_thickness_voxels }
            }
        }
    }

    /// Build a whole mask out of the active body's geometry.
    ///
    /// # It runs on this thread, and the plan asked for two halves
    ///
    /// Increment 25a specifies an `iced::Task` in two halves like
    /// `ImportRequested` / `ImportLoaded`, with a progress tick. **It is not
    /// written that way and the reason is the document, not the effort.** An
    /// import builds a fresh volume from a path, so the future owns everything
    /// it touches; a generator reads the volume the document is holding, and
    /// there is no way to lend that across an await. The three ways out are all
    /// worse than the hitch: cloning the volume is a second copy of up to four
    /// gigabytes, moving it out and back leaves the body missing from the
    /// document mid-flight -- visible, and editable -- and an `Arc<Volume>`
    /// rewrites every mutating path in the crate.
    ///
    /// What it costs, measured on the reference dragon: **39.6 ms on 24 cores**,
    /// against **71.0 ms for the whole-document remesh that has to follow it**
    /// and that is already synchronous. So moving only the pass off-thread would
    /// hide a third of the pause and buy a mid-flight state that nothing else in
    /// this application has. Extrapolated to the 45,567-brick ceiling the pass
    /// is about 265 ms, and if that becomes the complaint the answer is to move
    /// the remesh and the pass together, not the pass alone.
    ///
    /// The elapsed time goes to stderr past [`hani_core::SLOW_SPLIT`], which
    /// is the same instrument `apply_split` uses and for the same reason: the
    /// status bar is already carrying the result, and the perf readout cannot
    /// see a frame that never came.
    fn generate_mask(&mut self, which: MaskGenerator) {
        let body = self.doc.active();
        let recipe = self.recipe_for(which);
        let name = self.active_body_name();

        if let Some(why) = self.refuse_the_generator(recipe) {
            self.status = why;
            log::warn!("{}", self.status);
            return;
        }

        let started = web_time::Instant::now();
        let mask = self.doc.active_volume().generated_mask(recipe);
        let elapsed = started.elapsed();
        if mask.is_free() {
            // No entry, no `unsaved`, no remesh. A generator that found nothing
            // has not changed the document, and pushing an entry for it would
            // spend the undo budget on putting back what is already there.
            self.status = format!("{}: nothing on {name} matches", recipe.verb());
            return;
        }

        let previous = self.doc.active_volume_mut().replace_mask(mask);
        self.commit_whole_mask(body, previous);
        self.status = format!("{} on {name}", recipe.done());
        if elapsed > hani_core::SLOW_SPLIT {
            eprintln!("{} on {name} took {} ms", recipe.verb(), elapsed.as_millis());
        }
    }

    /// Why a generated mask would not fit under the memory ceiling.
    ///
    /// **The refusal is part of this feature rather than a nicety.** A
    /// hand-painted mask collapses to tiles almost everywhere; a generated one
    /// writes a value at every surface voxel, so the +25% of the mask
    /// arithmetic is what is actually paid -- and the thickness walk copies the
    /// field on top of that. Predicting it is
    /// `hani_core::Volume::generated_mask_demand`'s job and refusing on it is
    /// this one's.
    fn refuse_the_generator(&self, recipe: MaskRecipe) -> Option<String> {
        let volume = self.doc.active_volume();
        let extra = volume.generated_mask_demand(recipe);
        let (why, _coarser) =
            volume.mask().would_fit(extra, self.doc_stats.resident_bytes, self.doc.voxel_size())?;
        Some(format!("could not {} on {}: {why}", recipe.verb(), self.active_body_name()))
    }

    /// Protect the half of every visible body that a cut would have removed.
    ///
    /// **The plane is the cut's own and so is the gesture**: ctrl held at the
    /// end of a cut drag writes protection instead of taking material away.
    /// That is the whole of increment 25c's "no new gesture" -- the brick
    /// classification, the ray pair and the plane are all the ones `finish_cut`
    /// already had, and what changes is which field the half-space is written
    /// into.
    ///
    /// Across every DRAWN body, for the reason `Document::clip` gives: a cut is
    /// a line the user draws across what they can see, and a mask drawn the same
    /// way has to mean the same thing. One compound entry of N
    /// `Change::WholeMask`, so one ctrl+Z takes the whole gesture back.
    fn mask_cutter(&mut self, cutters: &[&[hani_core::ClipPlane]], shape: cut::CutShape) {
        let feather_mm = HALFSPACE_FEATHER_VOXELS * self.doc.voxel_size();
        let drawn = self.drawn_nodes();
        let bodies: Vec<NodeId> = self
            .doc
            .nodes()
            .iter()
            .enumerate()
            .filter(|(index, _)| drawn.get(*index).copied().unwrap_or(false))
            .map(|(_, node)| node.id)
            .collect();

        let active = self.doc.active();
        let mut changes = Vec::new();
        let mut touched = 0usize;
        // Two facts the old status line confused, and the confusion was the
        // whole of its second defect. This loop walks every DRAWN body, while
        // `split_the_mask_off` takes `doc.active()` and nothing else -- so "the
        // body this sentence is about" and "the body the split would take" are
        // not the same body. It named the second while describing the first.
        let mut only: Option<NodeId> = None;
        let mut touched_active = false;
        for body in bodies {
            let Some(volume) = self.doc.volume_mut(body) else {
                continue;
            };
            // Straight to `cutter_mask` rather than through `MaskRecipe`, and
            // that is deliberate. `MaskRecipe` derives `Copy` and travels
            // inside `Message`, so a slice in it would need a lifetime, an
            // `Arc` would cost the `Copy`, and a fixed array would put several
            // hundred bytes into every message the application sends. The
            // recipe stays the single-plane one it always was.
            let mask = volume.cutters_mask(cutters, feather_mm);
            if mask.is_free() {
                // The plane missed this body entirely, so it keeps whatever it
                // was carrying. Replacing it with an empty mask would throw a
                // hand-painted one away for a line drawn somewhere else.
                continue;
            }
            let previous = volume.replace_mask(mask);
            changes.push(Change::WholeMask { body, mask: Box::new(previous) });
            self.thumbs.geometry_changed(body);
            touched += 1;
            // `then_some` rather than a first-write guard: this clears itself
            // on the second body, so `only` is safe to read anywhere rather
            // than only under a `touched == 1` arm somewhere further down.
            only = (touched == 1).then_some(body);
            touched_active |= body == active;
        }

        if changes.is_empty() {
            self.status = "the mask gesture missed the model".to_string();
            return;
        }
        let before = self.history.stats();
        self.history.push(Entry::new(changes));
        self.record_history(before);
        self.unsaved = true;
        self.remesh_dirty();
        // Names what was protected, and then names the thing to do next.
        // **Circling a lump and being told only that it is masked is half an
        // answer**: the reason to select rather than cut is to lift the lump
        // off as its own body, and a user who is not told that is available
        // will reach for the panel and hunt for it.
        //
        // Which is exactly what happened. This used to say "split it off from
        // the BODIES panel", and the BODIES panel has no such button -- its
        // verb row is five icons and its Split fires `Message::BodySplit`, the
        // connected-component split, a different operation. See `split_advice`.
        let what = match shape {
            cut::CutShape::Line => "that half",
            cut::CutShape::Curve => "inside the curve",
            cut::CutShape::Lasso => "inside the lasso",
        };
        let where_ = match only {
            Some(body) => self.body_name(body),
            None => format!("{touched} bodies"),
        };
        self.status = match shape {
            cut::CutShape::Line => format!("masked {what} of {where_}"),
            _ => {
                format!("masked {what} on {where_} — {}", split_advice(touched_active, touched))
            }
        };
    }

    /// Split the active body in two along its mask.
    ///
    /// No preview card, unlike `split_active_body`: a masked split always makes
    /// exactly two bodies, so the question a card exists to answer -- how many
    /// parts is this -- is already answered, and the tint on screen is a better
    /// preview than any card could be.
    ///
    /// The three lines after the split are the unit `apply_split`,
    /// `Hani::remove_body` and `Hani::apply_merge` all document: the source
    /// stops existing, its bricks are in nobody's dirty set, so
    /// `SharedFrame::forget_body` is what releases its slots and the
    /// whole-document remesh is the other half.
    fn split_the_mask_off(&mut self) {
        let source = self.doc.active();
        let name = self.active_body_name();

        let visible = self
            .doc
            .index_of(source)
            .and_then(|index| self.shown.get(index).copied())
            .unwrap_or(true);
        if !visible {
            self.status = format!(
                "could not split {name}: it is not on screen, and its halves would not be either"
            );
            return;
        }
        if let Some(why) = self.doc.split_masked_guard(source) {
            self.status = format!("could not split {name}: {why}");
            return;
        }

        let started = web_time::Instant::now();
        let Some(outcome) = self.doc.split_masked(source) else {
            // The guard covers the two O(1) cases; this is the third, and it
            // needs the pass to find out: a mask that is neither empty nor
            // total can still sit entirely off the material.
            self.status = format!("the mask on {name} does not divide it");
            return;
        };
        self.shared.forget_body(source);
        self.doc.mark_everything_dirty();

        let before = self.history.stats();
        self.history.push(outcome.entry);
        self.unsaved = true;
        self.status = format!("split {name} along its mask");
        self.record_history(before);
        if started.elapsed() > hani_core::SLOW_SPLIT {
            eprintln!("masked split of {name} took {} ms", outcome.elapsed.as_millis());
        }

        self.refresh_model_radius();
        self.remesh_dirty();
        self.refresh_overlay();
    }

    /// Part the active body in two along a drawn line, with keys.
    ///
    /// The cut tool's own gesture with alt held: the plane is the one a cut
    /// would have used, and instead of a side going the body becomes two
    /// bodies with pegs on one and sockets in the other. See
    /// `hani_core::part`.
    ///
    /// **The ACTIVE body and no other**, where a cut crosses everything
    /// drawn. A cut is an eraser and parting is a decision about one part:
    /// what its two halves are called, where their keys go, what its
    /// neighbours do not get done to them. Every refusal names the body.
    ///
    /// The shape of everything after the plan is `split_the_mask_off`'s, for
    /// that function's reasons: the source's slots are released in the frame
    /// its halves are marked dirty, and the whole gesture is one undo entry.
    fn part_along(&mut self, planes: &[hani_core::ClipPlane], shape: cut::CutShape) {
        let source = self.doc.active();
        let name = self.active_body_name();
        let (cut::CutShape::Line, [plane]) = (shape, planes) else {
            self.status = format!("not parted: parting {name} takes one straight line across it");
            return;
        };

        let visible = self
            .doc
            .index_of(source)
            .and_then(|index| self.shown.get(index).copied())
            .unwrap_or(true);
        if !visible {
            self.status = format!(
                "could not part {name}: it is not on screen, and its halves would not be either"
            );
            return;
        }
        if let Some(why) = self.doc.split_guard(source) {
            self.status = format!("could not part {name}: {why}");
            return;
        }
        let Some(volume) = self.doc.volume(source) else {
            return;
        };

        // The plan first, and read only: it is what says whether the line
        // meets the body at all, and what goes in the status line about the
        // faces that could not be keyed.
        let plan = volume.part_plan(*plane, self.key_spec);
        if !plan.cuts() {
            self.status = format!("not parted: the line misses {name}");
            return;
        }
        let summary = plan.summary();
        let Some(outcome) = self.doc.part(source, &plan) else {
            // The plan found a face and the cut left one side empty, which a
            // plane grazing the body can do.
            self.status = format!("not parted: the line only grazes {name}");
            return;
        };
        self.shared.forget_body(source);
        self.doc.mark_everything_dirty();

        let before = self.history.stats();
        self.history.push(outcome.entry);
        self.unsaved = true;
        self.status = format!("parted {name} in two: {summary}");
        self.record_history(before);

        self.refresh_model_radius();
        self.remesh_dirty();
        self.refresh_overlay();
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

    /// Whether a modal card is up, and therefore owns the input.
    ///
    /// One list, read by both halves of the guard: `on_key` for the keyboard
    /// and `on_pointer` for the pointer. Before this existed the pointer had
    /// its own inline pair of checks and the keyboard had none at all, so
    /// `ctrl+Z` under the unsaved-work prompt changed the very volume the
    /// prompt was asking about, `x` flipped a mirror plane behind the card,
    /// and `1`-`6` swapped the brush. The bug-report dialog was in neither
    /// list, so a press beside its card sculpted.
    ///
    /// # Adding to this list is not automatically right
    ///
    /// It answers "is the document unreachable right now", and every card here
    /// is a question the user must answer before anything else happens.
    ///
    /// **This comment used to say that the split preview belonged in the
    /// keyboard guard and NOT the pointer one, and that whoever added it should
    /// split this function in two.** That was written against a split preview
    /// whose gesture was dragging a plane across the model. The split that
    /// landed is split by loose parts, and its card is a question with two
    /// buttons and no gesture at all -- so it belongs in both guards, as the
    /// plan for increment 16 asks, and the function stays one list.
    ///
    /// The warning still stands for whatever modeless overlay does eventually
    /// take the pointer: **split this function in two at that point rather than
    /// widening it and quietly killing the drag.**
    fn modal_open(&self) -> bool {
        self.welcome
            || self.preferences
            || self.shortcuts
            // See `app/help.rs`.
            || self.help.open
            // See `app/about.rs`.
            || self.about_open
            // See `app/shelf.rs`.
            || self.shelf.open
            // See `app/snip.rs`.
            || self.snipping()
            || self.confirm.is_some()
            || self.orient_prompt.is_some()
            || self.bug_report.is_some()
            || self.pending_delete.is_some()
            || self.pending_merge.is_some()
            || self.pending_split.is_some()
            // See `app/designer.rs`.
            || self.painting.designer.is_some()
    }

    /// What a key press means, now that nothing in the widget tree wanted it.
    ///
    /// The decode itself is `viewport::shortcut`, which is a pure function of
    /// the key and its modifiers so that it can be tested without a window —
    /// this is the half that needs the application, and it exists so that
    /// there is exactly one place where a key turns into a change.
    fn on_key(
        &mut self,
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> Task<Message> {
        match key {
            // Escape is the one key a modal must still see, so it sits above
            // the guard: it is how the unsaved-work prompt is cancelled and
            // the orientation prompt declined. It does NOT dismiss the bug
            // report, and that is on purpose -- the card holds a description
            // the user typed, and throwing it away on a stray Escape is the
            // same class of loss the confirm prompt exists to prevent.
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => {
                // A snip being chosen, over everything. See `app/snip.rs`.
                if self.snip_escaped() {
                    return Task::none();
                }
                // The welcome screen first, and unlike the bug report it DOES
                // go on Escape: it holds nothing the user typed, so there is
                // nothing to lose by dismissing it, and a modal with no
                // keyboard exit is the one thing worse than no modal.
                if self.welcome {
                    self.welcome = false;
                    return Task::none();
                }
                // The same for the two cards of the menus, which hold
                // nothing typed either.
                if self.shortcuts || self.preferences {
                    self.shortcuts = false;
                    self.preferences = false;
                    return Task::none();
                }
                // Help, as those. See `app/help.rs`.
                if self.help_escaped() {
                    return Task::none();
                }
                // About, as those. See `app/about.rs`.
                if self.about_escaped() {
                    return Task::none();
                }
                // And the browser's card. See `app/shelf.rs`.
                if self.shelf_escaped() {
                    return Task::none();
                }
                self.update(Message::MenuClosed)
            }
            iced::keyboard::Key::Character(character) => {
                // THE keyboard modal guard. Everything a shortcut can reach --
                // undo, the mirror planes, the brush numbers -- edits the
                // document or the tool behind a card that is asking about it.
                if self.modal_open() {
                    return Task::none();
                }
                let Some(message) = crate::viewport::shortcut(
                    character.as_str(),
                    modifiers.command(),
                    modifiers.shift(),
                    modifiers.alt(),
                ) else {
                    return Task::none();
                };
                // A key that means nothing in the open mode does nothing: W
                // is Transform's, C is Sculpt's and Print's. See
                // `app/workspace.rs`.
                if !self.desk.now.allows(&message) {
                    return Task::none();
                }
                // One level of recursion and never more: `shortcut` returns
                // ordinary messages and cannot return another `KeyPressed`.
                self.update(message)
            }
            // Tab: back to the workspace before this one, the one key that
            // makes Sculpt and Paint and Sculpt again a flick. Under a card
            // it does nothing, as every shortcut does.
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab)
                if !self.modal_open() && !modifiers.command() =>
            {
                self.update(Message::WorkspaceBack)
            }
            iced::keyboard::Key::Named(_) | iced::keyboard::Key::Unidentified => Task::none(),
        }
    }

    fn on_pointer(&mut self, event: PointerEvent) {
        // Fingers become the mouse's events, or the view's: `app/touch.rs`.
        if let PointerEvent::Touch { finger, phase, position, size } = event {
            self.touched(finger, phase, Vec2::new(position.x, position.y), size);
            return;
        }
        // While a modal prompt is up, the pointer belongs to it. The scrim in
        // `view` now genuinely swallows presses -- it is wrapped in
        // `iced::widget::opaque`, which captures them, where the bare styled
        // container it used to be forwarded every one -- and this is the other
        // half of that guarantee, because the viewport is a shader widget that
        // sees events wherever the cursor is. Both halves or neither: without
        // this a press behind the card would sculpt into a document the user
        // is about to discard, into a model they are about to turn, or into
        // the very state a bug report is describing.
        if self.modal_open() {
            // ...but a row picked up before the card went up has to be put
            // down, or the list stays armed and the NEXT press in the panel
            // arrives with a drag already in flight. Dropped rather than
            // performed: a move committed from behind a scrim is exactly the
            // edit this guard exists to refuse.
            if matches!(event, PointerEvent::Released { .. }) {
                self.row_drag = None;
            }
            return;
        }
        self.last_activity = Instant::now();
        match event {
            // Taken off at the top of this function.
            PointerEvent::Touch { .. } => {}
            PointerEvent::Modifiers { shift, control, alt } => {
                self.shift = shift;
                self.control = control;
                self.alt = alt;
                // Both change what a press would do, so the ring has to say so
                // before the press rather than after.
                self.refresh_overlay();
            }
            PointerEvent::Pressed { button, position, size } => {
                self.viewport_size = Vec2::new(size.x, size.y);
                let position = Vec2::new(position.x, position.y);
                self.cursor = Some(position);

                // **A second button down during a live transform abandons it.**
                //
                // There was no `self.drag.is_some()` guard here, so a right or
                // middle press assigned an Orbit or Pan drag straight over a
                // live `Transforming` one while `gizmo.grabbed` and the partial
                // `placement` survived. The right release then took the Orbit
                // arm, so the Transforming arm never ran and the left release
                // found no drag at all: the handle stayed lit, the preview box
                // went on drawing, and the gizmo visibly floated away from its
                // body.
                //
                // The abandoned gesture was not merely cosmetic. It is not
                // committed by the next `disarm_gizmo` -- that pushes the
                // pre-arm base against whatever the document already holds --
                // but by the NEXT gizmo drag's release, because that press
                // latches `pinned = placement` and adopts the stray placement
                // as its starting point.
                //
                // Cancelled exactly as Escape cancels, rather than ignoring the
                // press: ignoring it leaves the user in a modal drag with no
                // exit that anything on screen tells them about.
                if let Some(live) = &mut self.gizmo
                    && live.grabbed.is_some()
                    && self.drag.is_some_and(|drag| drag.button != button)
                {
                    live.placement = live.pinned;
                    live.grabbed = None;
                    self.drag = None;
                    self.refresh_gizmo();
                    self.status = "cancelled the drag".to_string();
                    return;
                }

                // An open menu swallows the next press: closing it is what the
                // click was for, and sculpting as well would be a surprise.
                if self.menu.take().is_some()
                    || self.top_menu.take().is_some()
                    || self.cube_menu.take().is_some()
                    || self.close_palette()
                {
                    return;
                }

                // A click while a measurement is being taken is one of its
                // two points, and not a stroke. See `app/measure.rs`.
                if self.measure_press(button, position) {
                    return;
                }
                // A press while the bodies are apart picks one. See
                // `app/explode.rs`.
                if self.explode_pressed(button, position) {
                    return;
                }
                // A press with the revolve's axis up draws its line. See
                // `app/revolve.rs`.
                if self.revolve_pressed(button, position) {
                    return;
                }

                // A press on the navigation cube belongs to the cube. Checked
                // before anything else, or clicking it would also carve a divot
                // out of the model behind it.
                //
                // Both buttons are taken, not just the left one. A right press
                // that fell through would start an orbit and then, on release,
                // open the brush menu on top of the cube -- and the cube's own
                // menu is what a right click there is for.
                if let Some(part) = navcube::pick(&self.camera, self.viewport_size, position) {
                    match button {
                        PointerButton::Left => self.fly_to(part),
                        PointerButton::Right => {
                            // Faces only. An edge or a corner points along no
                            // axis, and a quarter turn is the only kind of
                            // re-orientation offered, so there is nothing the
                            // menu could truthfully offer for one.
                            if part.extremes == 1
                                && let Some(facing) = hani_core::Facing::nearest(part.direction)
                            {
                                self.cube_menu = Some(CubeMenu { at: position, facing });
                            }
                        }
                        PointerButton::Middle => {}
                    }
                    return;
                }

                // **Before every other left-press rule, and bounds-checked
                // before it may claim anything.** ONLY BOUNDS-CHECKED EVENTS
                // MAY CAPTURE: `gizmo::pick` returns `None` for anything
                // outside the gizmo's own disc, so a press on the model still
                // reaches the sculpt below. It sits above the tool match rather
                // than inside `Tool::Transform`'s arm so that the gizmo is
                // reachable while the pointer is over another body -- selecting
                // a different body out from under an armed gizmo is exactly the
                // surprise the press ordering exists to remove.
                //
                // A held resize still outranks it, which is the precedence the
                // press dispatch below already settled between sizing and the
                // cut: a resize whose press turned into something else is the
                // worse of the two surprises, and the pointer belongs to that
                // gesture whether or not a button is down.
                if button == PointerButton::Left
                    && self.sizing.is_none()
                    && let Some(gizmo) = self.gizmo
                    && let Some(handle) =
                        gizmo::pick(&self.camera, self.viewport_size, &gizmo, position)
                {
                    if let Some(live) = &mut self.gizmo {
                        // Latched at the press: `drag` is absolute from this
                        // pixel and measures about the placement as it stands
                        // now, so both have to be held for the whole gesture.
                        live.pinned = live.placement;
                        live.grabbed = Some(handle);
                    }
                    self.drag = Some(Drag {
                        button,
                        kind: DragKind::Transforming(handle),
                        origin: position,
                        pressed_at: Instant::now(),
                        moved: false,
                        pivot: None,
                    });
                    self.refresh_gizmo();
                    return;
                }

                let kind = match button {
                    // Left sculpts -- unless a hold-and-drag resize is in
                    // progress, in which case the pointer belongs to that
                    // gesture and a press must not lay down a stroke.
                    //
                    // **This now outranks an armed cut, where it used to lose
                    // to one.** The cut and the mask collapsed into one match
                    // when `cut_armed` became a `Tool`, and the merged arm takes
                    // MASK's precedence rather than the cut's: a destructive
                    // one-shot may reasonably outrank a resize, an ongoing paint
                    // may not, and a resize whose press turned into a stroke is
                    // the worse of the two surprises.
                    PointerButton::Left if self.sizing.is_some() => DragKind::Sizing,
                    PointerButton::Left => match self.tool {
                        // The mode was chosen deliberately and the next left
                        // drag is the line, not a stroke.
                        Tool::Cut => {
                            // The press is the first point of the path, and the
                            // clear is what stops a cancelled gesture leaking
                            // into the next one.
                            self.cut_path.clear();
                            self.cut_path.push(position);
                            // Or the middle of a ring. See `app/ring_cut.rs`.
                            if self.rings() {
                                self.ring_pressed(position);
                            }
                            DragKind::Cutting
                        }
                        // Deliberately does NOT pick: masking paints the ACTIVE
                        // body, and a press that silently moved the target
                        // mid-mask would paint protection onto a body the user
                        // was not looking at. `refresh_hover` puts the ring on
                        // the active body while this tool is live, for the same
                        // reason and so that the two agree.
                        Tool::Mask => DragKind::Masking,
                        Tool::Sculpt => self.sculpt_press(position),
                        // The press missed every handle -- the gizmo claims its
                        // own above. Choosing a body is the useful thing left,
                        // and it re-arms on the new one, which is what makes
                        // "move this one now" one press rather than three.
                        Tool::Transform => {
                            let chosen = self.pick(position).map(|(body, _)| body);
                            match chosen {
                                Some(body) if body != self.doc.active() => {
                                    self.disarm_gizmo(true);
                                    // See `app/group_gizmo.rs`.
                                    self.body_picked();
                                    self.select_body(body);
                                    if let Err(why) = self.arm_gizmo() {
                                        self.status = why;
                                        self.tool = Tool::Sculpt;
                                    }
                                    DragKind::Selecting
                                }
                                _ => DragKind::Refused,
                            }
                        }
                    },
                    // Right and middle move the camera. Shift slides instead of
                    // turning.
                    PointerButton::Right | PointerButton::Middle => {
                        if self.shift {
                            DragKind::Pan
                        } else {
                            DragKind::Orbit
                        }
                    }
                };
                self.drag = Some(Drag {
                    button,
                    kind,
                    origin: position,
                    pressed_at: Instant::now(),
                    moved: false,
                    // `refresh_hover` ran on the motion event that brought the
                    // pointer here, so this costs a field read and no march.
                    //
                    // **The hover, with no fallback.** Orbiting about a point
                    // MOVES the target, so an anchor off the model walks the
                    // target sideways on every drag that started over empty
                    // background -- measured at 68 mm from one twenty step drag
                    // on a 30 mm ball, and it compounds. `None` here means
                    // "turn about the target", which is what orbiting has
                    // always done. [`Hani::navigation_anchor`] refuses an
                    // off-model anchor for the wheel for the same reason,
                    // arrived at separately and measured separately. Pinned by
                    // `an_orbit_drag_that_starts_off_the_model_turns_about_the_target`.
                    pivot: (kind == DragKind::Orbit).then_some(self.hover).flatten(),
                });

                let timed = self.phases.enter(phases::Phase::Edit);
                if let DragKind::Sculpt(direction) = kind {
                    self.sculpt_to(position, direction, true);
                }
                if kind == DragKind::Masking {
                    self.mask_to(position, true);
                }
                self.phases.leave(timed);
                self.refresh_hover(position);
                self.refresh_overlay();
            }
            PointerEvent::Released { button } => {
                // The revolve's line let go: it becomes a body. See
                // `app/revolve.rs`.
                if self.revolve_released(button) {
                    self.refresh_overlay();
                    return;
                }
                // **The one commit path for a row drag**, and it is the global
                // release rather than the row's own `on_release`: a row only
                // sees a release while the pointer is still over it, so a drag
                // that ended anywhere else would leave the list armed. See
                // `RowDrag`.
                if button == PointerButton::Left {
                    self.finish_row_drag();
                }
                if let Some(drag) = self.drag.filter(|drag| drag.button == button) {
                    if matches!(drag.kind, DragKind::Cutting) {
                        if self.rings() {
                            // The ring let go. See `app/ring_cut.rs`.
                            self.cut_path.clear();
                            self.ring_released();
                        } else {
                            // The drag already recorded where the button went down.
                            self.finish_cut(drag.origin);
                        }
                    }
                    // **The one place the field is rebuilt**, and it is the
                    // release rather than the motion: a bake per frame would be
                    // hundreds of trilinear passes for one gesture, and the
                    // preview box is what stands in for the model in the
                    // meantime.
                    if matches!(drag.kind, DragKind::Transforming(_)) {
                        if let Some(live) = &mut self.gizmo {
                            live.grabbed = None;
                        }
                        self.rebake_gizmo();
                        self.refresh_gizmo();
                    }
                    // A mask gesture ends the same way a sculpt one does, and
                    // through the same function: both opened the same recorder,
                    // and one gesture is one entry whichever of the two maps it
                    // wrote.
                    if matches!(drag.kind, DragKind::Sculpt(_) | DragKind::Masking) {
                        // What a drag that adds does when the button comes
                        // up: a cable's far end, or a body of its own.
                        self.add_released();
                        self.finish_stroke();
                    }
                    // A right press that neither moved nor lingered was a click,
                    // and a click opens the tool's own controls where the hand
                    // already is. Anything else was an orbit or a pan.
                    if button == PointerButton::Right
                        && !drag.moved
                        && drag.pressed_at.elapsed().as_millis() < CLICK_MS
                    {
                        self.menu = Some(drag.origin);
                    }
                    self.drag = None;
                }
                self.refresh_overlay();
            }
            PointerEvent::Moved { position, size } => {
                self.viewport_size = Vec2::new(size.x, size.y);
                let position = Vec2::new(position.x, position.y);
                let delta = self.cursor.map(|previous| position - previous).unwrap_or(Vec2::ZERO);
                self.cursor = Some(position);
                // The loose end of a measurement follows the pointer.
                self.measure_hover(position);
                // The revolve's line follows the pointer. See `app/revolve.rs`.
                self.revolve_moved(position);

                // Past the slop this is a drag, not a click, and it stays one
                // even if it comes back: otherwise an orbit that ends where it
                // began would open the menu.
                if let Some(drag) = &mut self.drag
                    && position.distance(drag.origin) > CLICK_SLOP_PX
                {
                    drag.moved = true;
                }

                // A held sizing key owns the pointer whether or not a button
                // is down, so this comes before the drag cases.
                if self.sizing.is_some() {
                    self.apply_sizing(position);
                    self.refresh_overlay();
                    return;
                }

                // The gizmo's hover, whether or not a button is down: it is
                // what says which handle a press would take, and it is a
                // handful of projections rather than a raycast.
                if let Some(gizmo) = self.gizmo {
                    let hovered = (self.drag.is_none())
                        .then(|| gizmo::pick(&self.camera, self.viewport_size, &gizmo, position))
                        .flatten();
                    if let Some(live) = &mut self.gizmo
                        && live.hovered != hovered
                    {
                        live.hovered = hovered;
                        self.refresh_gizmo();
                    }
                }

                match self.drag.map(|drag| drag.kind) {
                    Some(DragKind::Transforming(handle)) => {
                        self.transform_to(handle, position);
                        // Nothing else on this event: a gizmo drag owns the
                        // pointer, and the brush ring under it would be a ring
                        // on a surface that is about to move.
                        return;
                    }
                    Some(DragKind::Sculpt(direction)) => {
                        let timed = self.phases.enter(phases::Phase::Edit);
                        self.sculpt_to(position, direction, false);
                        self.phases.leave(timed);
                    }
                    Some(DragKind::Masking) => {
                        let timed = self.phases.enter(phases::Phase::Edit);
                        self.mask_to(position, false);
                        self.phases.leave(timed);
                    }
                    Some(DragKind::Orbit) => {
                        // About the surface the press landed on, so turning to
                        // look at the other side of a detail keeps the detail
                        // on screen instead of swinging it out of frame.
                        //
                        // **The fallback is the TARGET**, which is what
                        // orbiting has always turned about. An anchor off the
                        // model would put the pivot wherever the cursor
                        // happened to be over empty background, and since
                        // orbiting about it moves the target, every drag that
                        // started off the model would walk the target sideways
                        // for no benefit at all.
                        match self.drag.and_then(|drag| drag.pivot) {
                            Some(pivot) => self.camera.orbit_about(delta, pivot),
                            None => self.camera.orbit(delta),
                        }
                        self.publish_camera();
                    }
                    Some(DragKind::Pan) => {
                        self.camera.pan(delta, self.viewport_size.y);
                        self.publish_camera();
                    }
                    // The cut accumulates its path here and redraws the
                    // preview; the model is not touched until the button comes
                    // up, so the shape can be adjusted freely and what is drawn
                    // is what will go. A press that chose a body is over:
                    // dragging must not turn it into a stroke on a body the
                    // user has only just arrived at.
                    // A ring's radius. See `app/ring_cut.rs`.
                    Some(DragKind::Cutting) if self.rings() => self.ring_moved(position),
                    Some(DragKind::Cutting) => {
                        // Spaced rather than every event, so an identical
                        // looking stroke reduces to the same shape whether it
                        // was drawn quickly or slowly.
                        if self
                            .cut_path
                            .last()
                            .is_none_or(|last| last.distance(position) >= cut::CUT_PATH_SPACING_PX)
                        {
                            self.cut_path.push(position);
                            self.refresh_overlay();
                        }
                    }
                    Some(DragKind::Sizing)
                    | Some(DragKind::Selecting)
                    | Some(DragKind::Refused)
                    | None => {}
                }

                // Over the cube: light the part under the pointer, and draw no
                // brush ring, because a press there will not sculpt.
                let over_cube = navcube::pick(&self.camera, self.viewport_size, position);
                if over_cube != self.cube_hover {
                    self.cube_hover = over_cube;
                    self.refresh_cube();
                }
                if over_cube.is_some() {
                    self.hover = None;
                    self.hover_body = None;
                    self.refresh_overlay();
                    return;
                }

                // The ring follows the pointer, and the surface under it moves
                // as a stroke cuts into it.
                self.refresh_hover(position);
                self.refresh_overlay();
            }
            PointerEvent::Scrolled { amount, position, size } => {
                self.viewport_size = Vec2::new(size.x, size.y);
                let position = Vec2::new(position.x, position.y);
                // The anchor rule lives in `navigation_anchor`, and it is the
                // whole of the fix for "I cannot zoom in to work on a detail"
                // and for the drift that fixing it originally introduced.
                let factor = OrbitCamera::zoom_factor(amount);
                let anchor = self.navigation_anchor(factor);
                self.camera.zoom_by_about(factor, anchor);
                self.publish_camera();
                // The ring is pushed onto the surface it sits over and the
                // pick that placed it was taken at the old camera, so a zoom
                // leaves it stale. Cheap, and only on a real wheel notch.
                self.refresh_hover(position);
                self.refresh_overlay();
            }
        }
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
mod tests {
    use super::*;

    /// `update` returns a `Task` now. Tests do not run the iced runtime, so
    /// there is nothing to hand it to and dropping it is correct — but it must
    /// be dropped deliberately rather than by `#[allow]`, or a test that should
    /// have driven a dialog would pass silently.
    fn update(app: &mut Hani, message: Message) {
        let task = app.update(message);
        drop(task);
    }

    use crate::tablet::PenState;
    use hani_core::{BrushKind, Change};
    use iced::Vector;
    use std::time::Duration;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn centre_of_viewport() -> Vector {
        Vector::new(SIZE.x / 2.0, SIZE.y / 2.0)
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// Run a camera flight to completion in controlled time.
    ///
    /// `Message::Frame` scales by the real clock, and consecutive calls in a
    /// test are microseconds apart, so driving it that way would never arrive.
    fn finish_flight(app: &mut Hani) {
        for _ in 0..200 {
            if app.flight.is_none() {
                return;
            }
            app.advance_flight(16.0);
            app.publish_camera();
        }
        panic!("the flight never finished");
    }

    fn press(app: &mut Hani, at: Vector) {
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: at,
            size: SIZE,
        });
    }

    fn release(app: &mut Hani) {
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
    }

    /// The whole input to geometry path, with no window and no GPU: a press at
    /// the centre of the viewport must raycast onto the sphere, stamp the
    /// brush, and leave meshed bricks waiting for upload.
    #[test]
    fn a_press_at_the_centre_of_the_viewport_changes_the_model() {
        let mut app = app();
        let front = app.camera.eye().normalize() * MODEL_RADIUS_MM;
        let before = app.doc.active_volume().sample_world(front);

        press(&mut app, centre_of_viewport());

        assert!(app.perf.edit_ms > 0.0, "no edit was timed, so the raycast missed the sphere");
        assert!(app.perf.dirty_bricks > 0, "the stroke dirtied nothing");
        assert!(
            app.doc.active_volume().sample_world(front) < before,
            "adding clay should have pushed the field negative at the surface"
        );
    }

    /// The stats readout starts collapsed, and the icon is what brings it back.
    ///
    /// Pinned because "it is not covering the model" is the whole point of the
    /// control: a default flipped by an unrelated edit to the constructor would
    /// put seven lines of monospace back across the corner of the sculpt with
    /// nothing failing anywhere.
    #[test]
    fn the_stats_readout_starts_collapsed_and_the_icon_toggles_it() {
        let mut app = app();
        assert!(!app.stats_open, "the readout should not be over the model on launch");

        update(&mut app, Message::StatsToggled);
        assert!(app.stats_open, "pressing the info icon should show the numbers");

        update(&mut app, Message::StatsToggled);
        assert!(!app.stats_open, "pressing it again should put them away");
    }

    #[test]
    fn the_history_budget_is_reported_before_anything_is_drawn() {
        let app = app();
        assert!(
            app.history_stats.budget_bytes > 0,
            "the overlay would show a zero byte history budget until the first stroke"
        );
    }

    #[test]
    fn a_press_that_misses_the_model_changes_nothing() {
        let mut app = app();
        let bricks_before = app.doc.active_volume().brick_count();

        // The far corner of the viewport looks past the sphere into empty space.
        press(&mut app, Vector::new(2.0, 2.0));

        assert_eq!(
            app.doc.active_volume().brick_count(),
            bricks_before,
            "a miss must not allocate"
        );
        assert_eq!(app.perf.dirty_bricks, 0, "a miss must not schedule a remesh");
    }

    #[test]
    fn orbiting_moves_the_camera_without_touching_the_model() {
        let mut app = app();
        let yaw = app.camera.yaw;
        let bricks = app.doc.active_volume().brick_count();

        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Right,
            position: centre_of_viewport(),
            size: SIZE,
        });
        app.on_pointer(PointerEvent::Moved { position: Vector::new(460.0, 300.0), size: SIZE });

        assert_ne!(app.camera.yaw, yaw, "a right drag should have orbited");
        assert_eq!(app.doc.active_volume().brick_count(), bricks, "orbiting must not sculpt");
    }

    #[test]
    fn releasing_a_different_button_does_not_cancel_a_stroke() {
        let mut app = app();
        press(&mut app, centre_of_viewport());
        app.on_pointer(PointerEvent::Released { button: PointerButton::Right });
        assert!(app.drag.is_some(), "the left button drag should still be live");

        release(&mut app);
        assert!(app.drag.is_none());
    }

    #[test]
    fn a_finished_stroke_becomes_exactly_one_undo_entry() {
        let mut app = app();
        press(&mut app, centre_of_viewport());
        for offset in 1..8 {
            app.on_pointer(PointerEvent::Moved {
                position: centre_of_viewport() + Vector::new(offset as f32 * 6.0, 0.0),
                size: SIZE,
            });
        }
        assert_eq!(app.history_stats.undo_entries, 0, "history should wait for the button up");

        release(&mut app);
        assert_eq!(
            app.history_stats.undo_entries, 1,
            "a whole drag is one entry, not one per pointer event"
        );
    }

    /// The eviction that has to speak for itself.
    ///
    /// A per-operation prompt cannot catch this: two folder deletes that each
    /// pass the reclaim allowance on their own go on to evict each other, and
    /// the only moment anything knows the body has become unrecoverable is the
    /// eviction. The delete gesture itself does not exist yet, so the entry is
    /// built here by hand -- what is under test is the path from
    /// `HistoryStats::dropped_bodies` to something the user can read.
    #[test]
    fn an_eviction_that_takes_a_deleted_body_with_it_reaches_the_status_line() {
        let mut app = app();
        let mut second = Volume::new(app.doc.voxel_size());
        second.seed_sphere(Vec3::new(60.0, 0.0, 0.0), 10.0);
        let doomed = app.doc.add_body("Body 2", second);
        app.remesh_dirty();

        // An allowance of one byte, so the next push evicts the delete while
        // the stroke budget is nowhere near its ceiling.
        app.history = History::with_budgets(hani_core::DEFAULT_HISTORY_BUDGET, 1);
        let at = app.doc.index_of(doomed).expect("the second body");
        let node = app.doc.remove(at);
        app.history.push(Entry::new(vec![Change::NodeRemoved { at, node: Box::new(node) }]));
        assert!(app.status.is_empty(), "nothing has been evicted yet");

        press(&mut app, centre_of_viewport());
        release(&mut app);

        assert_eq!(app.history_stats.dropped_bodies, 1, "the delete was not evicted");
        assert!(
            app.status.contains("deleted body"),
            "the eviction left the user nothing to read: {:?}",
            app.status
        );
    }

    /// A body that comes back from the undo stack has to reach the GPU, and
    /// the count the remesh reports is the only thing in the application that
    /// says it did.
    ///
    /// `perf.dirty_bricks` and not `perf.remesh_ms`: the first is written
    /// before `remesh_dirty`'s early return and the second only past it, so a
    /// restored body that scheduled nothing would leave `remesh_ms` reading
    /// whatever the last real remesh cost.
    ///
    /// The remesh before the delete is load-bearing -- it is what drains the
    /// dirty set the second body was seeded with, so that the only thing that
    /// can mark its bricks afterwards is `Document::insert`.
    #[test]
    fn undoing_a_body_delete_schedules_every_one_of_its_bricks_for_remesh() {
        let mut app = app();
        let mut second = Volume::new(app.doc.voxel_size());
        second.seed_sphere(Vec3::new(60.0, 0.0, 0.0), 10.0);
        let doomed = app.doc.add_body("Body 2", second);
        app.remesh_dirty();

        let bricks = app.doc.volume(doomed).expect("the second body").brick_count();
        assert!(bricks > 0, "the fixture body must have bricks or this asserts nothing");

        // The delete itself, which has no interface yet: the node moves out of
        // the document and into the entry.
        let at = app.doc.index_of(doomed).expect("the second body");
        let node = app.doc.remove(at);
        app.history.push(Entry::new(vec![Change::NodeRemoved { at, node: Box::new(node) }]));

        update(&mut app, Message::Undo);

        assert!(app.doc.volume(doomed).is_some(), "the body did not come back");
        assert!(
            app.perf.dirty_bricks >= bricks,
            "the restored body scheduled {} bricks, not its {bricks}, so it never reaches the screen",
            app.perf.dirty_bricks
        );
    }

    #[test]
    fn undo_returns_the_model_to_where_it_started() {
        let mut app = app();
        let front = app.camera.eye().normalize() * MODEL_RADIUS_MM;
        let before = app.doc.active_volume().sample_world(front);

        press(&mut app, centre_of_viewport());
        release(&mut app);
        assert_ne!(app.doc.active_volume().sample_world(front), before);

        update(&mut app, Message::Undo);
        assert_eq!(
            app.doc.active_volume().sample_world(front),
            before,
            "undo did not restore the field"
        );
        assert!(app.perf.dirty_bricks > 0, "undo must schedule a remesh or the screen goes stale");

        update(&mut app, Message::Redo);
        assert_ne!(
            app.doc.active_volume().sample_world(front),
            before,
            "redo did not reapply the stroke"
        );
    }

    #[test]
    fn a_drag_stamps_more_than_once_along_its_path() {
        // Without interpolation a fast drag leaves a dotted trail, so check the
        // stroke actually produced several stamps from one pointer event.
        let mut app = app();
        app.brush.radius = 1.0;
        press(&mut app, centre_of_viewport());

        app.on_pointer(PointerEvent::Moved {
            position: centre_of_viewport() + Vector::new(80.0, 0.0),
            size: SIZE,
        });
        assert!(
            app.perf.stamps > 1,
            "one long pointer move should interpolate, got {} stamps",
            app.perf.stamps
        );
    }

    #[test]
    fn symmetry_sculpts_both_sides_at_once() {
        let mut app = app();
        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        // Nothing has told the application how big the viewport is yet, and
        // the ray depends on it.
        app.viewport_size = Vec2::new(SIZE.x, SIZE.y);

        // Aim off to one side so the mirrored half lands somewhere distinct.
        let off_centre = Vector::new(SIZE.x * 0.38, SIZE.y * 0.5);
        let hit = app
            .surface_under(Vec2::new(off_centre.x, off_centre.y))
            .expect("the test needs a point that is on the model");
        let mirrored = Vec3::new(-hit.x, hit.y, hit.z);
        let before = app.doc.active_volume().sample_world(mirrored);

        press(&mut app, off_centre);
        release(&mut app);

        assert!(
            app.doc.active_volume().sample_world(mirrored) < before,
            "the mirrored half of the stroke never landed"
        );
    }

    /// Six copies round Z: one press lays the stroke at every sixth of a
    /// turn round the axis through the middle, and not only where it was
    /// pressed. See `app/radial.rs`.
    #[test]
    fn radial_symmetry_lays_a_copy_at_every_turn() {
        let mut app = app();
        update(&mut app, Message::Radial(crate::app::RadialMessage::Copies(6.0)));
        update(&mut app, Message::Radial(crate::app::RadialMessage::Axis(MirrorAxis::Z)));
        app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
        let off_centre = Vector::new(SIZE.x * 0.5, SIZE.y * 0.3);
        let hit = app
            .surface_under(Vec2::new(off_centre.x, off_centre.y))
            .expect("the test needs a point that is on the model");
        let copies: Vec<Vec3> = (1..6)
            .map(|k| glam::Quat::from_rotation_z(std::f32::consts::TAU * k as f32 / 6.0) * hit)
            .collect();
        let before: Vec<f32> =
            copies.iter().map(|at| app.doc.active_volume().sample_world(*at)).collect();
        press(&mut app, off_centre);
        release(&mut app);
        for (k, (at, was)) in copies.iter().zip(before).enumerate() {
            let now = app.doc.active_volume().sample_world(*at);
            assert!(now < was, "copy {} at {at} never landed: {was} then {now}", k + 1);
        }
    }

    /// The ZBrush convention, and the biggest ergonomic win in this round.
    #[test]
    fn holding_shift_sculpts_with_smooth_without_changing_the_selection() {
        let mut app = app();
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        assert_eq!(app.effective_brush().kind, BrushKind::Draw);

        update(
            &mut app,
            Message::Pointer(PointerEvent::Modifiers { shift: true, control: false, alt: false }),
        );
        assert_eq!(app.effective_brush().kind, BrushKind::Smooth, "shift should smooth");
        assert_eq!(
            app.brush.kind,
            BrushKind::Draw,
            "the selection itself must not change, or the tool strip would flicker \
             and a key released out of focus would strand the wrong brush"
        );

        update(
            &mut app,
            Message::Pointer(PointerEvent::Modifiers { shift: false, control: false, alt: false }),
        );
        assert_eq!(app.effective_brush().kind, BrushKind::Draw, "releasing shift should restore");
    }

    #[test]
    fn holding_shift_actually_smooths_the_model_rather_than_drawing_on_it() {
        let mut app = app();
        // A move first, so the application knows the viewport size and
        // `surface_under` agrees with where a press will actually land. The
        // default size is not SIZE, and disagreeing here probes a point the
        // brush never touches.
        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        let probe = app
            .surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0))
            .expect("the centre of the viewport should hit the sphere");
        let flat = app.doc.active_volume().sample_world(probe);

        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        for _ in 0..4 {
            press(&mut app, centre_of_viewport());
            release(&mut app);
        }
        let raised = app.doc.active_volume().sample_world(probe);
        assert!(raised < flat, "drawing should have pushed the surface out past the probe");

        // Now the same gesture with shift held. Nothing about the selection
        // changes, but the strokes must smooth rather than pile on more.
        update(
            &mut app,
            Message::Pointer(PointerEvent::Modifiers { shift: true, control: false, alt: false }),
        );
        for _ in 0..12 {
            press(&mut app, centre_of_viewport());
            release(&mut app);
        }
        let smoothed = app.doc.active_volume().sample_world(probe);

        assert_ne!(smoothed, raised, "holding shift changed nothing at all");
        assert!(
            smoothed > raised,
            "shift should have flattened the bump back, not built on it: \
             {flat} flat, {raised} raised, {smoothed} after smoothing"
        );
        assert_eq!(app.brush.kind, BrushKind::Draw, "the selection must be untouched");
    }

    #[test]
    fn a_fresh_application_starts_with_every_mirror_plane_off() {
        let app = app();
        assert!(app.symmetry_settings().is_off(), "symmetry was {:?}", app.symmetry_settings());
        for axis in MirrorAxis::ALL {
            assert!(!app.symmetry_settings().axis(axis), "{} was on at startup", axis.label());
        }
    }

    #[test]
    fn each_mirror_axis_reaches_the_opposite_side() {
        for (axis, probe) in [
            (MirrorAxis::X, Vec3::new(-1.0, 1.0, 1.0)),
            (MirrorAxis::Y, Vec3::new(1.0, -1.0, 1.0)),
            (MirrorAxis::Z, Vec3::new(1.0, 1.0, -1.0)),
        ] {
            let mut app = app();
            update(&mut app, Message::SymmetryAxisToggled(axis));
            assert!(app.symmetry_settings().axis(axis), "{} did not turn on", axis.label());

            // On the surface, and off all three planes, so only the axis
            // under test can carry the stroke to the probe point.
            let at = Vec3::new(14.0, 14.0, 18.0).normalize() * MODEL_RADIUS_MM;
            let normal = app.doc.active_volume().gradient_world(at);
            let before = app.doc.active_volume().sample_world(at * probe);

            app.doc.active_volume_mut().begin_stroke();
            let symmetry = app.symmetry_settings();
            app.brush.apply_symmetric(
                app.doc.active_volume_mut(),
                &Stamp::new(at, normal, BrushDirection::Add),
                symmetry,
                MIRROR_CENTRE,
                &mut app.brush_scratch,
            );

            assert!(
                app.doc.active_volume().sample_world(at * probe) < before,
                "{} symmetry never reached the other side",
                axis.label()
            );

            // ...and toggling it again turns it back off.
            update(&mut app, Message::SymmetryAxisToggled(axis));
            assert!(!app.symmetry_settings().axis(axis));
        }
    }

    /// The fast path for radius in every sculpting tool is a drag, not a
    /// slider. Absolute against where the drag began, so out and back returns
    /// to exactly the original value rather than drifting.
    #[test]
    fn holding_the_sizing_key_and_dragging_scales_the_radius_reversibly() {
        let mut app = app();
        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        let original = app.brush.radius;

        update(&mut app, Message::SizingStarted(SizingTarget::Radius));
        assert!(app.sizing.is_some(), "the gesture did not start");

        let at = |dx: f32| Vector::new(SIZE.x / 2.0 + dx, SIZE.y / 2.0);
        app.on_pointer(PointerEvent::Moved { position: at(120.0), size: SIZE });
        let grown = app.brush.radius;
        assert!(grown > original, "dragging right should grow the brush");

        app.on_pointer(PointerEvent::Moved { position: at(-120.0), size: SIZE });
        assert!(app.brush.radius < original, "dragging left should shrink it");

        // Back where it started: absolute, not accumulated.
        app.on_pointer(PointerEvent::Moved { position: at(0.0), size: SIZE });
        assert!(
            (app.brush.radius - original).abs() < 1.0e-4,
            "the gesture drifted: {original} became {}",
            app.brush.radius
        );

        update(&mut app, Message::SizingEnded);
        assert!(app.sizing.is_none());
        // ...and the pointer goes back to sculpting.
        app.on_pointer(PointerEvent::Moved { position: at(200.0), size: SIZE });
        assert!((app.brush.radius - original).abs() < 1.0e-4, "sizing outlived its key");
    }

    /// The failure this would otherwise cause is the worst kind: a drag that
    /// silently carves the model while the user thinks they are resizing.
    #[test]
    fn a_sizing_drag_does_not_sculpt() {
        let mut app = app();
        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        let probe = app
            .surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0))
            .expect("the centre should hit the sphere");
        let before = app.doc.active_volume().sample_world(probe);

        update(&mut app, Message::SizingStarted(SizingTarget::Radius));
        press(&mut app, centre_of_viewport());
        for step in 1..=8 {
            app.on_pointer(PointerEvent::Moved {
                position: Vector::new(SIZE.x / 2.0 + step as f32 * 10.0, SIZE.y / 2.0),
                size: SIZE,
            });
        }
        release(&mut app);

        assert_eq!(
            app.doc.active_volume().sample_world(probe),
            before,
            "a sizing drag cut into the model"
        );
        assert!(!app.history.can_undo(), "a sizing drag recorded an undo entry");
    }

    #[test]
    fn the_sizing_gesture_stays_inside_the_slider_bounds() {
        // The radius ceiling now depends on the voxel size, so it is asked of a
        // real app rather than read off a constant.
        let radius_ceiling = app().max_radius();
        for (target, low, high) in [
            (SizingTarget::Radius, MIN_RADIUS_MM, radius_ceiling),
            (SizingTarget::Strength, MIN_STRENGTH, MAX_STRENGTH),
        ] {
            for direction in [-1.0f32, 1.0] {
                let mut app = app();
                app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
                update(&mut app, Message::SizingStarted(target));
                app.on_pointer(PointerEvent::Moved {
                    position: Vector::new(SIZE.x / 2.0 + direction * 5000.0, SIZE.y / 2.0),
                    size: SIZE,
                });
                let value = match target {
                    SizingTarget::Radius => app.brush.radius,
                    SizingTarget::Strength => app.brush.strength,
                };
                assert!((low..=high).contains(&value), "{target:?} left its range at {value}");
            }
        }
    }

    /// ZBrush calls this Dynamic. Without it a brush tuned on one model is the
    /// wrong size the moment the model changes scale.
    /// **A saved view that cannot see the model is re-framed rather than
    /// restored, and the near plane comes from the body rather than from the
    /// starting ball.**
    ///
    /// This is the regression for a real file. `Hani.hani` -- a 133 mm
    /// model centred on the origin -- was saved carrying `target` 435 mm away
    /// and `distance` 0.3 mm, and reported as "I cannot navigate in this file:
    /// not the SpaceMouse, not the scroll wheel, not the navigation cube."
    /// All three at once is the tell that nothing was wrong with any of them:
    /// pan, zoom and orbit all scale with `distance`, and 0.3 mm makes every
    /// one of them move by nothing you can see.
    ///
    /// 0.3 is exactly `near * 10.0` for a 30 mm ball, which is what
    /// `apply_view` used to spread over every restore regardless of what was
    /// in the file. So the clamp that should have stopped at 1.33 mm let the
    /// camera inside the model and then pinned it there, and the next save
    /// wrote that back to disk.
    ///
    /// Both halves are asserted, because either alone leaves the file unusable:
    /// the floor has to follow the body, and a poisoned view already on disk
    /// has to recover.
    #[test]
    fn a_saved_view_that_cannot_see_the_model_is_re_framed() {
        let mut app = app();
        app.doc.active_volume_mut().seed_sphere(Vec3::ZERO, 133.0);
        app.rebuild_everything();

        let poisoned = hani_core::View {
            camera_target: Vec3::new(-435.18, -12.09, 95.65),
            camera_distance: 0.3,
            camera_yaw: 42.6,
            camera_pitch: -0.05,
            ..hani_core::View::default()
        };
        app.apply_view(&poisoned);

        let radius = app.doc.active_volume().content_radius().expect("the ball has bounds");
        // **The floor assertion changed with the unit it is measured in, and
        // the behaviour it pinned is the thing being fixed.** It used to demand
        // `near * 10.0 > 1.0` -- 1.33 mm on this model -- as proof that the
        // floor had been computed from the body rather than from the 30 mm
        // starting ball. The floor is now two voxels, so it is the same
        // closeness on every model by construction and cannot come from the
        // wrong body at all; a fraction-of-the-radius floor was itself the
        // reason detail work was impossible. What is still worth pinning is
        // that the floor is a sane multiple of the lattice and that the
        // restored distance clears it.
        assert!(
            (app.camera.min_distance() - app.doc.voxel_size() * 2.0).abs() < 1.0e-6,
            "the zoom floor is {} on a {} mm lattice",
            app.camera.min_distance(),
            app.doc.voxel_size()
        );
        assert!(
            app.camera.target.distance(Vec3::ZERO) <= radius,
            "the camera is still aimed {} mm from a model of radius {radius}",
            app.camera.target.distance(Vec3::ZERO)
        );
        assert!(
            app.camera.distance >= app.camera.min_distance(),
            "the restored distance {} is below its own floor {}",
            app.camera.distance,
            app.camera.min_distance()
        );
        assert!(
            (app.camera.yaw - 42.6).abs() < 1.0e-3,
            "re-framing threw away the angles the user had set"
        );

        // And the recovery is visible rather than silent.
        assert!(
            app.status.contains("re-framed"),
            "the view moved and nothing said so: {}",
            app.status
        );

        // The whole point: a scroll now moves the camera by something you can see.
        let before = app.camera.distance;
        app.camera.zoom_by(OrbitCamera::zoom_factor(1.0));
        assert!(
            (app.camera.distance - before).abs() > 1.0,
            "one notch of scroll moved {} mm",
            (app.camera.distance - before).abs()
        );
    }

    /// The whole point of the format: sculpt, save, reopen, and get the same
    /// thing back. Driven through the application's own open and save rather
    /// than through the format directly, because the interesting failures are
    /// in the wiring -- a stale mesh pool, history that outlives its model, a
    /// camera that does not follow.
    #[test]
    fn a_sculpt_survives_a_save_and_a_reopen() {
        let directory = std::env::temp_dir().join(format!("hani-roundtrip-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("sculpt.hani");

        let mut app = app();
        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        let probe = app
            .surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0))
            .expect("the centre should hit the sphere");

        // Make something worth keeping, and move the camera and settings so the
        // session state has something to prove.
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        for _ in 0..4 {
            press(&mut app, centre_of_viewport());
            release(&mut app);
        }
        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::Y));
        update(&mut app, Message::BrushRadiusChanged(5.5));
        app.camera.yaw = 1.25;
        let sculpted = app.doc.active_volume().sample_world(probe);
        let expected_bricks: usize = app.doc.active_volume().brick_coords().count();

        app.save_project(&path);
        assert!(app.status.starts_with("saved"), "save reported: {}", app.status);
        assert_eq!(app.project_path.as_deref(), Some(path.as_path()));

        // A different session entirely, as reopening after quitting would be.
        let mut reopened = Hani::with_tablet(crate::tablet::Tablet::inert());
        reopened.open_project(&path);
        assert!(reopened.status.starts_with("opened"), "open reported: {}", reopened.status);

        assert_eq!(
            reopened.doc.active_volume().sample_world(probe),
            sculpted,
            "the field came back different"
        );
        assert_eq!(reopened.doc.active_volume().brick_coords().count(), expected_bricks);
        assert!((reopened.camera.yaw - 1.25).abs() < 1.0e-5, "the camera did not follow");
        assert!((reopened.brush.radius - 5.5).abs() < 1.0e-5, "the brush did not follow");
        assert!(
            reopened.symmetry_settings().axis(MirrorAxis::Y),
            "the mirror planes did not follow"
        );

        // And it was drawn and is printable, which is what "loaded" means.
        // Note `open_project` meshes immediately, so the dirty set is already
        // drained by now -- what proves it happened is how much it meshed.
        assert!(
            reopened.perf.dirty_bricks > 0,
            "the reopened model was never meshed, so it would load into an empty screen"
        );
        let (_, report) = reopened.doc.active_volume().export_mesh();
        assert!(report.is_printable(), "the reopened model is not printable: {}", report.summary());

        // History belongs to the model that went away.
        assert!(!reopened.history.can_undo(), "undo outlived the model it belonged to");

        std::fs::remove_dir_all(&directory).ok();
    }

    // --- the unsaved marker --------------------------------------------------
    //
    // Quitting with unsaved work used to lose it silently. These pin the flag
    // that the confirm prompt reads, and in particular the three places it must
    // NOT be set, since a flag that is always on is the same as no flag at all.

    #[test]
    fn a_stroke_marks_the_document_unsaved_and_a_save_clears_it() {
        let directory = std::env::temp_dir().join(format!("hani-unsaved-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("sculpt.hani");

        let mut app = app();
        assert!(!app.unsaved, "a freshly opened sphere is not unsaved work");

        press(&mut app, centre_of_viewport());
        release(&mut app);
        assert!(app.unsaved, "a stroke did not mark the document unsaved");

        app.save_project(&path);
        assert!(app.status.starts_with("saved"), "save reported: {}", app.status);
        assert!(!app.unsaved, "a successful save did not clear the marker");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A press and release that never met the model produces no undo entry, so
    /// it must produce no prompt either. Without the guard inside
    /// `finish_stroke` every stray click in empty space would arm the marker.
    #[test]
    fn a_press_that_misses_the_model_does_not_mark_it_unsaved() {
        let mut app = app();
        press(&mut app, Vector::new(4.0, 4.0));
        release(&mut app);
        assert!(
            !app.unsaved,
            "a click into empty space armed the unsaved marker, so every stray \
             click would raise a discard prompt"
        );
        assert!(
            !app.history.can_undo(),
            "the corner of the viewport hit the model after all, so this test \
             proves nothing -- move the press"
        );

        // The control: the same gesture at the centre does arm it, so the
        // assertion above is about the miss and not about a press path that
        // silently does nothing.
        press(&mut app, centre_of_viewport());
        release(&mut app);
        assert!(app.unsaved, "the control press missed too, so the test is vacuous");
    }

    /// A write that failed did not save anything, so the marker has to survive
    /// it. Clearing it in the wrong place lets the next quit go through with no
    /// prompt and no file.
    #[test]
    fn a_failed_save_leaves_it_unsaved() {
        let mut app = app();
        press(&mut app, centre_of_viewport());
        release(&mut app);
        assert!(app.unsaved);

        let nowhere = std::env::temp_dir()
            .join(format!("hani-absent-{}", std::process::id()))
            .join("no")
            .join("such")
            .join("directory")
            .join("sculpt.hani");
        app.save_project(&nowhere);

        assert!(app.status.contains("could not"), "a failed save reported: {}", app.status);
        assert!(app.unsaved, "a failed save cleared the unsaved marker");
        assert!(app.project_path.is_none(), "a failed save claimed the file");
    }

    /// The puck's buttons call `undo`/`redo` directly rather than sending
    /// `Message::Undo`, so the marker has to be set in the methods. Calling them
    /// the way `drive_from_spacemouse` does is what proves that.
    #[test]
    fn undo_and_redo_mark_it_unsaved_through_the_direct_call_path() {
        let mut app = app();
        press(&mut app, centre_of_viewport());
        release(&mut app);
        app.unsaved = false;

        app.undo();
        assert!(app.unsaved, "undo did not mark the document unsaved");

        app.unsaved = false;
        app.redo();
        assert!(app.unsaved, "redo did not mark the document unsaved");
    }

    /// Undo with nothing to undo changes nothing, so it must not arm the marker
    /// either -- the flag lives inside the `history.undo` guard for this.
    #[test]
    fn undo_with_an_empty_history_does_not_mark_it_unsaved() {
        let mut app = app();
        assert!(!app.history.can_undo(), "the fixture already had something to undo");
        app.undo();
        assert!(!app.unsaved, "an undo that did nothing armed the unsaved marker");

        // The control. Without it this test asserts the default value and would
        // pass just as happily if `undo` did nothing at all, in either case.
        press(&mut app, centre_of_viewport());
        release(&mut app);
        app.unsaved = false;
        app.undo();
        assert!(
            app.unsaved,
            "an undo with something to undo failed to mark it, so the \
                              assertion above was vacuous"
        );
    }

    /// `resample` returns early when the size is already in use. The marker sits
    /// past that return, so the no-op stays a no-op.
    #[test]
    fn resampling_to_the_current_size_leaves_the_flag_alone() {
        let mut app = app();
        app.resample(app.doc.voxel_size());
        assert!(!app.unsaved, "a resample that did nothing armed the unsaved marker");

        app.resample(app.doc.voxel_size() * 2.0);
        assert!(app.unsaved, "a real resample did not mark the document unsaved");
    }

    #[test]
    fn the_title_carries_the_file_name_and_the_unsaved_star() {
        let mut app = app();
        assert_eq!(app.title(), "untitled — HaniSculpt");

        press(&mut app, centre_of_viewport());
        release(&mut app);
        assert_eq!(app.title(), "untitled* — HaniSculpt");

        app.project_path = Some(std::path::PathBuf::from("/tmp/dragon.hani"));
        assert_eq!(app.title(), "dragon.hani* — HaniSculpt");

        app.unsaved = false;
        assert_eq!(app.title(), "dragon.hani — HaniSculpt");
    }

    // --- the confirm-before-discard prompt -----------------------------------

    /// An app with a stroke in it and nowhere to save it, which is the state
    /// every one of these is about.
    fn app_with_unsaved_work() -> Hani {
        let mut app = app();
        press(&mut app, centre_of_viewport());
        release(&mut app);
        assert!(app.unsaved, "the fixture did not actually dirty the document");
        app
    }

    #[test]
    fn new_with_unsaved_work_raises_the_prompt_and_does_not_reset() {
        let mut app = app_with_unsaved_work();
        let sculpted = app.doc.active_volume().brick_coords().count();

        update(&mut app, Message::NewSculpt);

        assert_eq!(app.confirm, Some(PendingAction::NewSculpt), "no prompt was raised");
        assert!(app.unsaved, "the document was reset behind the prompt");
        assert_eq!(
            app.doc.active_volume().brick_coords().count(),
            sculpted,
            "New threw the sculpt away before the user had answered"
        );
    }

    /// The panel's Reset sphere button shares an arm with File > New, so it
    /// discards the document identically and must ask identically.
    #[test]
    fn reset_sphere_asks_as_well_since_it_shares_the_arm() {
        let mut app = app_with_unsaved_work();
        update(&mut app, Message::ResetSphere);
        assert_eq!(app.confirm, Some(PendingAction::NewSculpt));
    }

    #[test]
    fn quitting_with_unsaved_work_raises_the_prompt() {
        let mut app = app_with_unsaved_work();
        let id = iced::window::Id::unique();
        update(&mut app, Message::CloseRequested(id));
        assert_eq!(
            app.confirm,
            Some(PendingAction::Quit(id)),
            "the close request was not intercepted, so quitting still loses work"
        );
    }

    /// With nothing to lose, every gated action must behave exactly as it did
    /// before the gate existed.
    #[test]
    fn a_clean_document_is_not_prompted_at_all() {
        let mut app = app();
        assert!(!app.unsaved, "the fixture starts dirty, so this proves nothing");

        update(&mut app, Message::NewSculpt);
        assert!(app.confirm.is_none(), "a clean document was prompted");

        update(&mut app, Message::CloseRequested(iced::window::Id::unique()));
        assert!(app.confirm.is_none(), "a clean document was prompted on quit");

        update(&mut app, Message::OpenRequested);
        assert!(app.confirm.is_none(), "a clean document was prompted on open");

        // The control: the same three actions DO prompt once there is something
        // to lose. Without it every assertion above is satisfied by the default.
        for action in [Message::NewSculpt, Message::CloseRequested(iced::window::Id::unique())] {
            let mut dirty = app_with_unsaved_work();
            update(&mut dirty, action);
            assert!(
                dirty.confirm.is_some(),
                "the gate never fires, so the checks above are vacuous"
            );
        }
    }

    /// The prompt draws over the viewport but the shader widget still sees
    /// events wherever the cursor is. A press behind the card must neither
    /// dismiss the prompt nor sculpt into a document about to be discarded.
    #[test]
    fn a_viewport_press_does_not_dismiss_the_prompt_or_sculpt() {
        let mut app = app_with_unsaved_work();
        update(&mut app, Message::NewSculpt);
        let before = app.doc.active_volume().sample_world(Vec3::ZERO);
        let entries = app.history_stats.undo_entries;

        press(&mut app, centre_of_viewport());
        release(&mut app);

        assert_eq!(app.confirm, Some(PendingAction::NewSculpt), "a stray click dismissed it");
        assert_eq!(
            app.doc.active_volume().sample_world(Vec3::ZERO),
            before,
            "it sculpted behind the prompt"
        );
        assert_eq!(
            app.history_stats.undo_entries, entries,
            "it recorded a stroke behind the prompt"
        );
    }

    /// Escape answers Cancel. It must not fall through to the menu clears,
    /// which would drop the prompt without answering it.
    #[test]
    fn escape_cancels_the_prompt_rather_than_dropping_it() {
        let mut app = app_with_unsaved_work();
        update(&mut app, Message::NewSculpt);

        update(&mut app, Message::MenuClosed);

        assert!(app.confirm.is_none(), "escape left the prompt up");
        assert!(app.unsaved, "escape discarded the work it was asking about");
        assert!(app.history.can_undo(), "escape reset the sculpt");
    }

    // --- the subscription's key decode ---------------------------------------
    //
    // Everything in the section below this one synthesises `Message::KeyPressed`
    // directly, which is downstream of `key_event` and cannot see it. These
    // four cases are the only thing standing between a wrong `key_event` and a
    // green suite: with the press arm deleted the whole keyboard is dead and
    // every guard test still passes, guarding a message nothing can produce.

    /// A window event carrying `character`, pressed or released.
    ///
    /// The fields beyond `key` and `modifiers` are what a real winit press
    /// fills in and `key_event` ignores; they are here because the literal
    /// does not compile without them.
    fn key_window_event(character: &str, modifiers: iced::keyboard::Modifiers) -> iced::Event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character(character.into()),
            modified_key: iced::keyboard::Key::Character(character.into()),
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    fn key_release_event(character: &str) -> iced::Event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
            key: iced::keyboard::Key::Character(character.into()),
            modified_key: iced::keyboard::Key::Character(character.into()),
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::empty(),
        })
    }

    /// `listen_with` hands every event to `key_event` regardless of who wanted
    /// it, so dropping the captured ones is the whole of the focus awareness.
    /// Inverting this check is verbatim the bug that shipped for a year:
    /// `1`-`7`, `s`, `u`, `x`, `y` and `z` stolen from every text field,
    /// because a focused input reports its keystrokes captured and the
    /// shortcut fired anyway.
    #[test]
    fn a_key_a_widget_already_consumed_never_reaches_the_application() {
        let typed = key_window_event("z", command());

        let message = key_event(typed, iced::event::Status::Captured, iced::window::Id::unique());

        assert!(
            message.is_none(),
            "a captured key press was forwarded, so shortcuts fire while typing"
        );
    }

    /// The other half: an event nobody claimed becomes a `KeyPressed` carrying
    /// exactly the key and modifiers that arrived. `on_key` decodes from those
    /// two fields alone, so anything lost here is a shortcut that cannot fire
    /// -- dropping the modifiers would turn every `ctrl+z` into a bare `z`.
    #[test]
    fn an_ignored_key_press_is_forwarded_with_its_key_and_modifiers_intact() {
        let mut modifiers = command();
        modifiers.insert(iced::keyboard::Modifiers::SHIFT);
        let pressed = key_window_event("z", modifiers);

        let message = key_event(pressed, iced::event::Status::Ignored, iced::window::Id::unique());

        let Some(Message::KeyPressed { key, modifiers: forwarded }) = message else {
            panic!("an ignored key press produced {message:?} rather than a KeyPressed");
        };
        assert_eq!(key, iced::keyboard::Key::Character("z".into()));
        assert!(forwarded.command(), "the command modifier was dropped");
        assert!(forwarded.shift(), "the shift modifier was dropped");
    }

    /// A left press the widget tree ignored becomes the blur signal, and a
    /// press anyone claimed does not.
    ///
    /// **This arm is the only thing in the application that can see a blur.**
    /// `text_input` unfocuses itself on any left press outside its own bounds
    /// and publishes nothing (`text_input.rs:723-735`), and the bodies list is
    /// a fixed six or eight rows tall, so a press in the empty scrollable
    /// under the last row belongs to no widget. Without this arm that press
    /// produced no message at all, `renaming` stayed set, and the field went
    /// on being drawn while the next keystroke fired a tool shortcut.
    ///
    /// Both controls matter. Captured must stay dropped, or clicking INSIDE
    /// the field would close it. The right button must stay silent, because
    /// `text_input` does not blur on it -- committing there would close a
    /// field that is still focused and still taking keys.
    #[test]
    fn a_left_press_nobody_wanted_is_the_only_blur_the_application_can_see() {
        let press = |button| iced::Event::Mouse(iced::mouse::Event::ButtonPressed(button));

        let ignored = key_event(
            press(iced::mouse::Button::Left),
            iced::event::Status::Ignored,
            iced::window::Id::unique(),
        );
        assert!(
            matches!(ignored, Some(Message::PressedNothing)),
            "a press on nothing produced {ignored:?}, so a rename in flight never hears about it"
        );

        let captured = key_event(
            press(iced::mouse::Button::Left),
            iced::event::Status::Captured,
            iced::window::Id::unique(),
        );
        assert!(
            captured.is_none(),
            "a press a widget wanted produced {captured:?}; clicking inside the field closes it"
        );

        let right = key_event(
            press(iced::mouse::Button::Right),
            iced::event::Status::Ignored,
            iced::window::Id::unique(),
        );
        assert!(
            right.is_none(),
            "a right press produced {right:?}, but the field stays focused through one"
        );
    }

    /// Releases are the one thing `key_event` decodes itself, because ending a
    /// gesture must not go through the modal guard. Only the two sizing keys
    /// mean anything on release; every other release is somebody else's.
    #[test]
    fn releasing_a_sizing_key_ends_the_gesture_and_no_other_release_means_anything() {
        for sizing in ["s", "u"] {
            let message = key_event(
                key_release_event(sizing),
                iced::event::Status::Ignored,
                iced::window::Id::unique(),
            );
            assert!(
                matches!(message, Some(Message::SizingEnded)),
                "releasing {sizing} produced {message:?} rather than ending the sizing drag"
            );
        }

        // And the mask's peek key, which is held for the same reason and needs
        // its release for a sharper one: a peek that never ends leaves the tint
        // stuck on with nothing the user can do about it but press again.
        let message = key_event(
            key_release_event("h"),
            iced::event::Status::Ignored,
            iced::window::Id::unique(),
        );
        assert!(
            matches!(message, Some(Message::MaskPeekEnded)),
            "releasing h produced {message:?} rather than ending the peek"
        );

        let message = key_event(
            key_release_event("x"),
            iced::event::Status::Ignored,
            iced::window::Id::unique(),
        );
        assert!(
            message.is_none(),
            "releasing x produced {message:?}; only presses carry meaning for the mirror keys"
        );
    }

    // --- the modal keyboard and pointer guard --------------------------------

    /// Press a character key the way the subscription does, on an event the
    /// widget tree ignored.
    ///
    /// Note what this does NOT do: call `viewport::shortcut` and feed the
    /// result in. That would test the decode and skip the guard, which is the
    /// only thing these tests are about.
    fn key(app: &mut Hani, character: &str, modifiers: iced::keyboard::Modifiers) {
        update(
            app,
            Message::KeyPressed {
                key: iced::keyboard::Key::Character(character.into()),
                modifiers,
            },
        );
    }

    fn bare() -> iced::keyboard::Modifiers {
        iced::keyboard::Modifiers::empty()
    }

    /// The modifier that `command()` reads, for whichever platform is running.
    ///
    /// **Not a bare `CTRL`.** The shortcut table is fed `modifiers.command()`,
    /// which iced maps to Cmd on macOS and Control everywhere else, so a test
    /// that sets the raw control bit proves nothing on a Mac -- it asserts that
    /// Control+Z undoes, which on that platform it correctly does not. Four
    /// tests failed exactly that way the first time this crate was built for
    /// macOS. The behaviour was right and the fixture was wrong.
    fn command() -> iced::keyboard::Modifiers {
        if cfg!(target_os = "macos") {
            iced::keyboard::Modifiers::LOGO
        } else {
            iced::keyboard::Modifiers::CTRL
        }
    }

    /// **An action from the welcome screen dismisses it.**
    ///
    /// It is a starting point and not a mode: pressing New sculpt has answered
    /// the question the screen was asking. And because the welcome card ranks
    /// above every other card, leaving it up draws the unsaved-work prompt that
    /// `New sculpt` raises UNDERNEATH it -- raised, unanswerable and invisible.
    /// Seen exactly that way in the running application before this landed.
    #[test]
    fn an_action_from_the_welcome_screen_dismisses_it_so_its_prompt_is_reachable() {
        let mut app = app_with_unsaved_work();
        app.welcome = true;

        update(&mut app, Message::NewSculpt);

        assert!(!app.welcome, "the welcome screen survived an action taken from it");
        assert!(
            app.confirm.is_some(),
            "the fixture raised no prompt, so this does not test what it names"
        );
    }

    /// The tick must not dismiss the screen it is a setting for, and neither
    /// must reading an article.
    ///
    /// **Asked of the rule and not by sending the message**, deliberately.
    /// `WelcomeOnStartupSet` writes through to the real
    /// `$XDG_CONFIG_HOME/hanisculpt/welcome.conf`, so driving it here would
    /// reach into the config of whoever is running the suite -- and two such
    /// tests in parallel would race on one file. `dismisses_the_welcome` is a
    /// pure function of the message, so it can be asked directly; that the
    /// preference round-trips is
    /// `welcome::tests::the_preference_survives_being_written_and_read_back`,
    /// which uses a path of its own for the same reason.
    /// **`with_devices` must not resolve the executable, and the crash-revert
    /// offer must therefore not be decided there.**
    ///
    /// This pins a trap that already cost the feature. The offer was originally
    /// decided at the tail of `with_devices`, guarded on `app.exe` -- which
    /// `new` does not assign until after `with_devices` has returned. So `exe`
    /// was unconditionally `None` at that point, the guard never passed, and
    /// "Go back to the previous build" could not appear in any build ever
    /// shipped. Correct logic, unit-tested in isolation, whose caller never ran.
    ///
    /// The suite could not have caught it, because ~640 tests build through
    /// `with_devices` and it leaves `exe: None` by design -- resolving a path
    /// and sweeping a directory is startup work, not test-construction work.
    /// So the invariant is pinned from the other side: if someone later makes
    /// `with_devices` resolve `exe`, this fails and they are pointed at the
    /// decision in `new` that assumes it does not.
    #[test]
    fn with_devices_leaves_the_executable_unresolved_so_new_decides_the_revert() {
        let app = Hani::with_tablet(Tablet::inert());
        assert!(
            app.exe.is_none(),
            "`with_devices` must not resolve the executable -- `new` does, and the crash-revert \
             offer is decided there because it needs it"
        );
        assert!(app.crash_revert.is_none(), "and nothing may offer a revert before that");
        assert!(!app.crashed_last_session, "a scratch-dir construction has no crash report");
    }

    /// **Rebuilding the view keeps solo, where rebuilding everything drops it.**
    ///
    /// That single difference is the whole reason these are two functions, so
    /// it is the only assertion here that discriminates -- and saying so
    /// matters, because the obvious companions do not. Asserting the pool reset
    /// was requested passes under either function, and so does asserting that
    /// the bricks were dirtied: both do those two things identically. A test
    /// built out of those would be green with `Message::ViewRebuilt` wired
    /// straight to `rebuild_everything`, which would silently throw away a
    /// user's solo mode every time they pressed a button about the mesh pool.
    #[test]
    fn rebuilding_the_view_packs_the_pool_without_dropping_solo() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let cube = app.doc.active();
        app.solo = Some(cube);
        // Clear whatever is pending, so the assertions below are about THIS
        // call and not about the primitive that was just added -- adding a body
        // dirties bricks too, and a counter left carrying its value made the
        // dirty assertion below pass with the remesh removed.
        app.shared.take_pool_reset_for_tests();
        app.perf.dirty_bricks = 0;

        update(&mut app, Message::ViewRebuilt);

        assert_eq!(
            app.solo,
            Some(cube),
            "a rebuild of the VIEW dropped solo -- that is `rebuild_everything`'s behaviour, and \
             it is right there only because a document swap renumbers the rows"
        );
        assert!(
            app.shared.take_pool_reset_for_tests(),
            "the pool was never asked to reset, so nothing was recovered"
        );
        // **The reset is only sound paired with marking everything dirty** --
        // `request_pool_reset`'s own doc says the model otherwise leaves the
        // GPU and does not come back. That is a different mutation from the
        // solo one above and it needs its own assertion: dropping the two
        // statements after the reset would leave the solo check green and the
        // screen empty.
        assert!(
            app.perf.dirty_bricks > 0,
            "the pool was emptied and nothing was re-offered, so the model has left the screen"
        );
        assert!(
            app.status.contains("rebuilt the view"),
            "the button reported nothing: {}",
            app.status
        );
    }

    /// The remedy must not claim to give memory back.
    ///
    /// `MeshPool::reset` empties the allocators over the buffers it already
    /// holds and never drops a `BufferPair`, so nothing is returned to the
    /// system. A sentence that read as freeing VRAM would be false in the one
    /// place a user is deciding whether they have a memory problem.
    #[test]
    fn the_rebuild_does_not_claim_to_have_freed_memory() {
        let mut app = app();
        update(&mut app, Message::ViewRebuilt);
        for lie in ["MB", "memory", "VRAM", "freed"] {
            assert!(
                !app.status.contains(lie),
                "the status implies memory came back, and it did not: {}",
                app.status
            );
        }
    }

    /// **An update refusal reaches the welcome card, not only the status line.**
    ///
    /// The install button lives on the welcome card, and that card draws over a
    /// full-window scrim and returns before every other card in `view` -- so
    /// the status line it reported into was behind the very screen the button
    /// is on. That is why the Windows relaunch failure has been reported more
    /// than once and its text never read. `update_failure` is the field the
    /// card draws; if a refusal does not reach it, the refusal is invisible
    /// again on exactly the screen that caused it.
    #[test]
    fn an_update_refusal_reaches_the_card_that_covers_the_status_line() {
        let mut app = app();

        // Through a real message, not by calling the setter: what this pins is
        // that the DISPATCH ARM routes to the writer. Calling `update_shows`
        // here would exercise a two-line setter against itself and stay green
        // if every arm went back to assigning `self.status` directly, which is
        // exactly the defect being fixed.
        update(
            &mut app,
            Message::UpdateInstalled(
                iced::window::Id::unique(),
                Err("starting the new build failed (os error 5)".to_string()),
            ),
        );

        assert_eq!(
            app.update_failure.as_deref(),
            Some("starting the new build failed (os error 5)"),
            "the welcome card has nothing to draw, so the reason is invisible on the screen the \
             install button is on"
        );
        assert_eq!(app.status, "starting the new build failed (os error 5)");
        assert!(!app.installing_update, "the install was left marked in flight");
    }

    /// Every refusal in the install path reaches the card, not just the first.
    ///
    /// **One test per arm, because each is a separate `let ... else` and the
    /// silent one was found by review rather than by this suite.** Removing
    /// any single `update_says` leaves the others green, so a per-arm
    /// assertion is the only shape that catches it. The middle arm is the one
    /// that returned in silence: unticking Check for updates clears `offer`
    /// while leaving `downloaded_update`, and the Help entry is gated on the
    /// download alone.
    #[test]
    fn every_refusal_in_the_install_path_reaches_the_card() {
        // No payload downloaded, reached through the message arm the Help
        // entry and the card's button both raise.
        {
            let mut nothing = app();
            update(&mut nothing, Message::UpdateInstallRequested(iced::window::Id::unique()));
            assert_eq!(
                nothing.update_failure.as_deref(),
                Some("download the update first"),
                "the guard in the message arm said nothing on the card"
            );
        }

        // And the same state reaching the install path itself, which has its
        // own sentence and its own arm.
        {
            let mut nothing = app();
            let _ = nothing.install_and_restart(iced::window::Id::unique());
            assert_eq!(
                nothing.update_failure.as_deref(),
                Some("there is no downloaded update to install"),
                "a press with nothing downloaded said nothing on the card"
            );
        }

        // A payload, but the offer behind it has been withdrawn.
        {
            let mut withdrawn = app();
            withdrawn.downloaded_update = Some(std::path::PathBuf::from("/tmp/does-not-matter"));
            withdrawn.offer = None;
            let _ = withdrawn.install_and_restart(iced::window::Id::unique());
            assert_eq!(
                withdrawn.update_failure.as_deref(),
                Some("the update was withdrawn -- switch checking back on and look again"),
                "the withdrawn-offer arm returned in silence and blanked the card"
            );
        }
    }

    /// A fresh attempt clears the last one's refusal, so the card cannot go on
    /// showing a reason that has been superseded.
    ///
    /// **The refusal has to be one this attempt cannot itself produce**, or the
    /// assertion is satisfied by the next statement rather than by the clear.
    /// The first draft used a string the very next `let ... else` overwrote, so
    /// it passed with the clear deleted.
    #[test]
    fn starting_another_install_clears_the_previous_refusal() {
        let mut app = app();
        app.update_shows("a reason from last time".to_string());

        // Reaches the first refusal, which writes its OWN string. If the clear
        // were removed the old reason would still be gone -- so instead: park
        // the app where every arm is silent, and check the field is empty.
        app.downloaded_update = Some(std::path::PathBuf::from("/tmp/does-not-matter"));
        app.offer = None;
        app.installing_update = false;
        app.update_failure = Some("a reason from last time".to_string());
        let _ = app.install_and_restart(iced::window::Id::unique());
        assert_ne!(
            app.update_failure.as_deref(),
            Some("a reason from last time"),
            "the previous attempt's reason survived a new attempt"
        );
    }

    /// **A second press while a swap is running changes nothing at all.**
    ///
    /// The guard has to come above the clear, not below it: clearing the note
    /// and then returning would wipe "installing build N…" off the card and
    /// leave a running install saying nothing on screen -- which is the
    /// "clicked install, nothing happened" report this path exists to stop
    /// producing.
    #[test]
    fn a_second_install_press_during_a_swap_does_not_wipe_what_the_card_is_saying() {
        let mut app = app();
        app.installing_update = true;
        app.update_failure = Some("installing build 1042…".to_string());
        app.status = "installing build 1042…".to_string();

        let _ = app.install_and_restart(iced::window::Id::unique());

        assert_eq!(
            app.update_failure.as_deref(),
            Some("installing build 1042…"),
            "a second press blanked the card while the install was still running"
        );
        assert_eq!(app.status, "installing build 1042…");
    }

    /// Choosing Export from the File menu closes the menu; the menu stayed
    /// over the viewport, and in a browser over its file chooser.
    #[test]
    fn a_file_menu_entry_chosen_closes_the_menu() {
        let mut app = app();
        app.welcome = false;
        update(&mut app, Message::TopMenuToggled(crate::message::TopMenu::File));
        assert_eq!(app.top_menu, Some(crate::message::TopMenu::File));
        update(&mut app, Message::ExportRequested(ExportFormat::Stl));
        assert_eq!(app.top_menu, None);
        assert!(!closes_the_file_menu(&Message::TopMenuToggled(crate::message::TopMenu::File)));
    }

    #[test]
    fn the_screens_own_controls_do_not_dismiss_it() {
        assert!(!dismisses_the_welcome(&Message::WelcomeOnStartupSet(false)));
        // The update tick sits on the same card and must behave the same way.
        // It is deliberately in none of the three classifiers, exactly like the
        // tick beside it: an unlisted variant commits an in-flight rename and
        // disarms the gizmo, and that is what the sibling control already does.
        assert!(!dismisses_the_welcome(&Message::UpdateCheckSet(false)));
        assert!(!dismisses_the_welcome(&Message::UpdateCheckSet(true)));
        assert!(!dismisses_the_welcome(&Message::WelcomeOnStartupSet(true)));
        assert!(
            !dismisses_the_welcome(&Message::LinkOpened(crate::links::RELEASE_PAGE.into())),
            "opening the release page closed the screen it was opened from"
        );

        // And the ones that must: every button that starts real work.
        assert!(dismisses_the_welcome(&Message::NewSculpt));
        assert!(dismisses_the_welcome(&Message::OpenRequested));
        assert!(dismisses_the_welcome(&Message::ImportRequested));
        assert!(dismisses_the_welcome(&Message::WelcomeClosed));
    }

    /// No shortcut may reach the document through the welcome screen.
    ///
    /// It is modal like every other card, and `modal_open` is the one place
    /// that is decided -- a card missing from that list is a card the keyboard
    /// reaches straight through.
    #[test]
    fn no_shortcut_fires_while_the_welcome_screen_is_up() {
        let mut app = app();
        app.welcome = true;
        let before = app.brush.kind;

        key(&mut app, "3", bare());
        key(&mut app, "x", bare());

        assert_eq!(app.brush.kind, before, "a brush number changed the tool behind the screen");
        assert!(
            !app.symmetry_settings().axis(MirrorAxis::X),
            "a mirror plane toggled behind the screen"
        );
        assert!(app.welcome, "a shortcut dismissed the screen instead of being swallowed");
    }

    /// Escape closes it, unlike the bug report, which holds typed text.
    #[test]
    fn escape_closes_the_welcome_screen() {
        let mut app = app();
        app.welcome = true;
        update(
            &mut app,
            Message::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                modifiers: bare(),
            },
        );
        assert!(!app.welcome, "Escape left the screen up, so it has no keyboard exit");
    }

    /// The undo that used to reach through the card.
    ///
    /// `Message::Undo` had no guard of any kind, so ctrl+Z with the
    /// unsaved-work prompt up rolled back the very stroke the prompt was
    /// asking whether to keep -- and answering Save then wrote out a document
    /// the user had not agreed to.
    #[test]
    fn control_z_under_a_modal_leaves_the_document_alone() {
        let mut app = app_with_unsaved_work();
        let front = app.camera.eye().normalize() * MODEL_RADIUS_MM;
        update(&mut app, Message::NewSculpt);
        assert!(app.confirm.is_some(), "the fixture never raised a prompt");

        let before = app.doc.active_volume().sample_world(front);
        key(&mut app, "z", command());

        assert_eq!(
            app.doc.active_volume().sample_world(front),
            before,
            "ctrl+Z undid a stroke behind the prompt"
        );
        assert!(app.history.can_undo(), "the stroke left the undo stack");
        assert!(app.confirm.is_some(), "the prompt went away on its own");
    }

    /// Every other shortcut, on every modal. The brush and the mirror planes
    /// are not the document, but changing them behind a card the user is
    /// reading is the same surprise, and `1`-`6` did exactly that.
    #[test]
    fn no_shortcut_fires_while_any_modal_card_is_up() {
        fn check(what: &str, raise: impl FnOnce(&mut Hani)) {
            let mut app = app_with_unsaved_work();
            let brush = app.brush.kind;
            raise(&mut app);
            assert!(app.modal_open(), "{what} did not count as a modal");

            key(&mut app, "x", bare());
            key(&mut app, "2", bare());

            assert!(
                !app.symmetry_settings().axis(MirrorAxis::X),
                "x flipped a mirror plane behind {what}"
            );
            assert_eq!(app.brush.kind, brush, "a digit swapped the brush behind {what}");
        }

        check("the unsaved-work prompt", |app| update(app, Message::NewSculpt));
        check("the bug report", |app| update(app, Message::BugReportOpened));
        check("the orientation prompt", |app| {
            app.adopt_import(imported_with(Some(hani_core::Facing::Back)));
        });
    }

    /// The control for both tests above: with nothing modal up, the same three
    /// keystrokes all land. Without this they would pass against a build where
    /// the keyboard was simply dead.
    #[test]
    fn the_same_keys_all_land_with_no_modal_up() {
        let mut app = app_with_unsaved_work();
        let front = app.camera.eye().normalize() * MODEL_RADIUS_MM;
        let sculpted = app.doc.active_volume().sample_world(front);
        assert!(!app.modal_open(), "the fixture is not a fair control");

        key(&mut app, "x", bare());
        assert!(app.symmetry_settings().axis(MirrorAxis::X), "x did not reach the mirror planes");

        key(&mut app, "2", bare());
        assert_eq!(app.brush.kind, BrushKind::ALL[1], "the digit did not reach the brush");

        // The mirror is the last step now, an undo step of its own (see
        // `app/symmetry_frame.rs`), so the first ctrl+Z takes it back and the
        // second the stroke.
        key(&mut app, "z", command());
        assert!(!app.symmetry_settings().axis(MirrorAxis::X), "ctrl+Z did not undo the mirror");
        key(&mut app, "z", command());
        assert_ne!(
            app.doc.active_volume().sample_world(front),
            sculpted,
            "ctrl+Z did not undo the stroke"
        );
    }

    /// Escape is the exception, and has to stay one: it is how the card is
    /// answered, so it is the one key that must pass the guard.
    #[test]
    fn escape_still_reaches_a_modal_through_the_key_path() {
        let mut app = app_with_unsaved_work();
        update(&mut app, Message::NewSculpt);

        update(
            &mut app,
            Message::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                modifiers: bare(),
            },
        );

        assert!(app.confirm.is_none(), "escape was swallowed by the modal guard");
        assert!(app.unsaved, "escape discarded the work it was asking about");
    }

    /// The bug-report card was in neither guard: the pointer early return
    /// named `confirm` and `orient_prompt` only, so a press beside the card
    /// carved the model the report was about while the user described it.
    #[test]
    fn a_press_beside_the_bug_report_card_does_not_sculpt() {
        let mut app = app();
        update(&mut app, Message::BugReportOpened);
        assert!(app.bug_report.is_some(), "the dialog never opened");

        let front = app.camera.eye().normalize() * MODEL_RADIUS_MM;
        let before = app.doc.active_volume().sample_world(front);
        press(&mut app, centre_of_viewport());
        release(&mut app);

        assert_eq!(
            app.doc.active_volume().sample_world(front),
            before,
            "a press reached the model behind the card"
        );
        assert!(app.drag.is_none(), "it started a stroke behind the card");
        assert!(!app.unsaved, "it dirtied a document nobody had touched");
    }

    #[test]
    fn discard_runs_the_pending_action() {
        let mut app = app_with_unsaved_work();
        update(&mut app, Message::NewSculpt);
        update(&mut app, Message::ConfirmAnswered(ConfirmChoice::Discard));

        assert!(app.confirm.is_none());
        assert!(!app.unsaved, "the fresh sphere is not unsaved work");
        assert!(!app.history.can_undo(), "the reset did not happen");
    }

    #[test]
    fn cancel_leaves_everything_exactly_as_it_was() {
        let mut app = app_with_unsaved_work();
        let bricks = app.doc.active_volume().brick_coords().count();
        update(&mut app, Message::NewSculpt);
        update(&mut app, Message::ConfirmAnswered(ConfirmChoice::Cancel));

        assert!(app.confirm.is_none());
        assert!(app.unsaved, "cancel cleared the unsaved marker");
        assert_eq!(
            app.doc.active_volume().brick_coords().count(),
            bricks,
            "cancel reset the sculpt anyway"
        );
    }

    /// Save-then-continue, for a document that already has a file.
    #[test]
    fn saving_from_the_prompt_writes_the_file_and_then_acts() {
        let directory =
            std::env::temp_dir().join(format!("hani-confirm-save-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("sculpt.hani");

        let mut app = app_with_unsaved_work();
        app.save_project(&path);
        assert!(!app.unsaved);

        // Dirty it again so the prompt has something to be about.
        press(&mut app, Vector::new(SIZE.x / 2.0 + 12.0, SIZE.y / 2.0));
        release(&mut app);
        assert!(app.unsaved);

        update(&mut app, Message::NewSculpt);
        update(&mut app, Message::ConfirmAnswered(ConfirmChoice::Save));

        assert!(app.confirm.is_none(), "the prompt stayed up after a good save");
        assert!(!app.history.can_undo(), "the pending New did not run after the save");
        // And the file on disk is the sculpt as it was, not the fresh sphere.
        let mut reopened = Hani::with_tablet(crate::tablet::Tablet::inert());
        reopened.open_project(&path);
        assert!(reopened.status.starts_with("opened"), "open reported: {}", reopened.status);

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A save that fails must not be treated as a save. Continuing here would
    /// quit on a file that was never written, which is the exact loss the
    /// prompt exists to prevent.
    #[test]
    fn a_failed_save_from_the_prompt_does_not_continue() {
        let mut app = app_with_unsaved_work();
        app.project_path = Some(
            std::env::temp_dir()
                .join(format!("hani-absent-{}", std::process::id()))
                .join("nope")
                .join("sculpt.hani"),
        );

        update(&mut app, Message::NewSculpt);
        update(&mut app, Message::ConfirmAnswered(ConfirmChoice::Save));

        assert!(app.status.contains("could not"), "reported: {}", app.status);
        assert_eq!(
            app.confirm,
            Some(PendingAction::NewSculpt),
            "the prompt was dismissed by a save that did not happen"
        );
        assert!(app.unsaved, "a failed save cleared the marker");
        assert!(app.history.can_undo(), "the sculpt was discarded on a failed save");
    }

    /// Dismissing the file dialog is not an answer to the prompt.
    #[test]
    fn dismissing_the_save_dialog_leaves_the_prompt_up() {
        let mut app = app_with_unsaved_work();
        update(&mut app, Message::NewSculpt);
        update(&mut app, Message::SavedThenContinue(None));

        assert_eq!(app.confirm, Some(PendingAction::NewSculpt));
        assert!(app.history.can_undo(), "the sculpt was discarded");
    }

    // --- the recent list and the crash net -----------------------------------

    /// A scratch directory for one test's recent list and autosave, so nothing
    /// here can reach the real ones.
    fn scratch(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!("hani-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn saving_and_opening_both_record_into_the_recent_list() {
        let directory = scratch("recent-record");
        let path = directory.join("sculpt.hani");

        let mut app = app_with_unsaved_work();
        app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
        app.save_project(&path);
        assert_eq!(app.recent.paths().len(), 1, "a save did not record the file");

        let mut reopened = Hani::with_tablet(crate::tablet::Tablet::inert());
        reopened.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
        reopened.open_project(&path);
        assert!(reopened.status.starts_with("opened"), "open reported: {}", reopened.status);
        assert_eq!(reopened.recent.paths()[0], std::path::absolute(&path).unwrap());

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A file that has been moved or deleted should stop being offered, rather
    /// than sitting in the menu failing every time it is clicked.
    #[test]
    fn opening_a_missing_recent_file_drops_it_from_the_list() {
        let directory = scratch("recent-missing");
        let gone = directory.join("gone.hani");

        let mut app = app();
        app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
        app.recent.record(&gone);
        assert_eq!(app.recent.paths().len(), 1);

        app.open_project(&gone);
        assert!(app.status.contains("could not"), "reported: {}", app.status);
        assert!(app.recent.is_empty(), "a file that cannot be opened stayed in the list");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A crash net this build cannot read is kept, not overwritten.
    ///
    /// **This is a data-loss guard, and the loss is silent.** The updater
    /// supports downgrade, so an older build can meet a newer build's autosave;
    /// it refuses to open it, says so, and then the 120-second timer replaces
    /// the only copy of the user's work with whatever is on screen.
    ///
    /// The unreadable file is manufactured by bumping the FIELD version in the
    /// header of a real autosave, which is exactly the shape a downgrade
    /// produces -- rather than by writing rubbish, which would prove only that
    /// rubbish is refused.
    #[test]
    fn an_autosave_this_build_cannot_read_is_kept_rather_than_overwritten() {
        let directory = scratch("autosave-unreadable");
        let path = directory.join("autosave.hani");

        // A genuine autosave first, so the bytes are real in every other way.
        let mut app = app_with_unsaved_work();
        app.autosave_file = Some(path.clone());
        app.write_autosave();
        let good = std::fs::read(&path).expect("the autosave should be there");

        // Bytes 10..12 are the field version. One past what this build can READ
        // is a file from a newer build -- and that is NOT one past what it just
        // wrote: an unpainted, unmasked document deliberately stamps the OLDEST
        // encoding that can carry it, which is the whole backward-compatibility
        // mechanism. So the boundary is found rather than assumed, which also
        // stops this test rotting at the next field bump.
        let mut from_the_future = good.clone();
        let mut version = u16::from_le_bytes([good[10], good[11]]);
        loop {
            version += 1;
            assert!(version < 64, "no field version in range was refused");
            from_the_future[10..12].copy_from_slice(&version.to_le_bytes());
            if hani_core::project::read_outline(&mut from_the_future.as_slice()).is_err() {
                break;
            }
        }
        std::fs::write(&path, &from_the_future).expect("write failed");

        let mut older = Hani::with_tablet(crate::tablet::Tablet::inert());
        older.autosave_file = Some(path.clone());
        let aside = older.preserve_unreadable_autosave().expect("it should have been kept");

        assert!(
            !path.exists(),
            "the unreadable autosave was left where the timer would clobber it"
        );
        assert!(aside.exists(), "it was moved somewhere that does not exist");
        assert_eq!(
            std::fs::read(&aside).expect("read failed"),
            from_the_future,
            "the kept file is not the bytes that were there"
        );
        assert_eq!(aside.file_name().unwrap(), "autosave-unreadable-1.hani");

        // A second one does not overwrite the first.
        std::fs::write(&path, &from_the_future).expect("write failed");
        let second = older.preserve_unreadable_autosave().expect("the second should be kept too");
        assert_eq!(second.file_name().unwrap(), "autosave-unreadable-2.hani");
        assert!(aside.exists(), "the second refusal destroyed the first one");

        // And a file this build CAN read is left exactly where it is.
        std::fs::write(&path, &good).expect("write failed");
        assert_eq!(older.preserve_unreadable_autosave(), None, "a good autosave was moved aside");
        assert!(path.exists(), "a readable autosave was taken away from File > Recover");

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn the_autosave_round_trips_but_is_not_a_save() {
        let directory = scratch("autosave");
        let mut app = app_with_unsaved_work();
        app.autosave_file = Some(directory.join("autosave.hani"));
        let probe = Vec3::ZERO;
        let sculpted = app.doc.active_volume().sample_world(probe);

        app.write_autosave();

        assert!(app.has_autosave(), "nothing was written");
        assert!(app.unsaved, "the autosave cleared the unsaved marker, so quitting would not ask");
        assert!(app.project_path.is_none(), "the autosave claimed to be the document's file");

        let mut recovered = Hani::with_tablet(crate::tablet::Tablet::inert());
        recovered.autosave_file = app.autosave_file.clone();
        recovered.recover_autosave();

        assert_eq!(
            recovered.doc.active_volume().sample_world(probe),
            sculpted,
            "the field came back different"
        );
        assert!(
            recovered.project_path.is_none(),
            "a recovered autosave adopted its own path, so the next Save would write back into \
             the crash net instead of asking for a real file"
        );
        assert!(recovered.unsaved, "a recovered autosave must still want saving somewhere real");
        assert!(recovered.recent.is_empty(), "the crash net was listed as a recent document");

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn an_explicit_save_throws_the_crash_net_away() {
        let directory = scratch("autosave-cleared");
        let mut app = app_with_unsaved_work();
        app.autosave_file = Some(directory.join("autosave.hani"));
        app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
        app.write_autosave();
        assert!(app.has_autosave());

        app.save_project(&directory.join("sculpt.hani"));
        assert!(app.status.starts_with("saved"), "save reported: {}", app.status);
        assert!(
            !app.has_autosave(),
            "a stale crash net survived a real save, so File > Recover would offer to restore \
             something older than the file on disk"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    /// **A failed save must leave the file it was replacing exactly as it
    /// was.** Until this increment the save opened the user's real file with
    /// `File::create`, which truncates it to zero before a byte of the new
    /// document is written -- so a failure part way through left the work in
    /// neither place.
    ///
    /// The failure is forced by putting a DIRECTORY where the temporary file
    /// wants to be, which is the cheapest way to make `File::create` fail
    /// without depending on permissions, a full disk or a mocked filesystem.
    #[test]
    fn a_save_that_fails_leaves_the_previous_file_and_the_crash_net_alone() {
        let directory = scratch("save-atomic");
        let path = directory.join("sculpt.hani");
        std::fs::write(&path, b"the previous save, which must survive").unwrap();

        let mut app = app_with_unsaved_work();
        app.autosave_file = Some(directory.join("autosave.hani"));
        app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
        app.write_autosave();
        assert!(app.has_autosave(), "the fixture needs a crash net to protect");

        // Where `save_project` will try to put its temporary.
        std::fs::create_dir_all(path.with_extension("hani.tmp")).unwrap();

        app.save_project(&path);

        assert!(app.status.contains("could not"), "a failed save reported: {}", app.status);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"the previous save, which must survive",
            "a failed save destroyed the file it was replacing"
        );
        assert!(app.unsaved, "a failed save cleared the unsaved marker");
        assert!(app.project_path.is_none(), "a failed save adopted the path anyway");
        assert!(app.has_autosave(), "a failed save deleted the crash net");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A save that works leaves the file and nothing else -- no temporary
    /// beside it for the user to find and wonder about.
    #[test]
    fn a_save_that_works_leaves_no_temporary_behind() {
        let directory = scratch("save-tidy");
        let path = directory.join("sculpt.hani");
        let mut app = app_with_unsaved_work();
        app.autosave_file = Some(directory.join("autosave.hani"));
        app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));

        app.save_project(&path);

        assert!(app.status.starts_with("saved"), "save reported: {}", app.status);
        assert!(path.is_file(), "the file was not written");
        assert!(
            !path.with_extension("hani.tmp").exists(),
            "the temporary the save writes through was left on disk"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    /// The save reads its own output back before it deletes the crash net.
    ///
    /// `clear_autosave` used to run on the writer's word alone: a write that
    /// reported success and produced a file this build will not open took the
    /// user's work, cleared the unsaved marker and deleted the one copy that
    /// could have recovered it. The check is the header and the node table
    /// only, which is a few hundred bytes whatever the sculpt weighs.
    ///
    /// Driven here by changing the document *after* the file is written, which
    /// is the same disagreement a partial write would produce and the only one
    /// a test can stage without a mocked filesystem.
    #[test]
    fn the_save_verification_notices_a_file_that_is_not_this_document() {
        let directory = scratch("save-verify");
        let path = directory.join("sculpt.hani");
        let mut app = app_with_unsaved_work();
        app.autosave_file = Some(directory.join("autosave.hani"));
        app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));

        app.save_project(&path);
        assert!(app.status.starts_with("saved"), "save reported: {}", app.status);
        assert!(app.verify_written(&path).is_ok(), "a file just written failed its own check");

        // One more body than the file on disk knows about.
        app.doc.add_body("Body 2", hani_core::Volume::new(app.doc.voxel_size()));
        let problem = app.verify_written(&path).expect_err("a stale file passed the check");
        assert!(problem.contains("2"), "the mismatch did not say what it found: {problem}");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// **`save_project` really returns on that verification** -- the test above
    /// proves the check works, not that anything calls it.
    ///
    /// That distinction was measured, not assumed: deleting the whole
    /// `if let Err(problem) = self.verify_written(&temporary)` gate from
    /// `save_project` left all 279 tests in this crate green. A repair that is
    /// real, correct, unit-tested and never called is a shape this project has
    /// shipped before, and the gate is the one standing between an unreadable
    /// file and `clear_autosave`.
    ///
    /// Driving it needs a temporary that swallows every write and hands back
    /// something else on the read, and `/dev/null` is exactly that: the save
    /// opens the symlink, `project::write` reports success at every step, and
    /// the read-back finds an empty file. Nothing is written through the link
    /// that survives, and the cleanup unlinks the LINK -- `remove_file` does
    /// not follow one -- so `/dev/null` itself is never touched.
    ///
    /// Rejected first: making the document disagree with the file, the trick
    /// the test above uses. It cannot work here, because `save_project` writes
    /// and verifies against the same `self.doc` within one call, so the two can
    /// never disagree without an injection point that does not exist.
    #[test]
    // The trick IS the symlink: a link to `/dev/null` is what makes the write
    // succeed and the read-back come up empty. Windows has neither piece, and
    // the behaviour under test -- keep the previous file when a save cannot be
    // read back -- is platform-independent, so it is covered here and taken on
    // trust there rather than faked with a second, weaker fixture.
    #[cfg(unix)]
    fn a_save_that_cannot_read_its_own_output_back_keeps_the_previous_file() {
        let directory = scratch("save-unreadable");
        let path = directory.join("sculpt.hani");
        std::fs::write(&path, b"the previous save, which must survive").unwrap();

        let mut app = app_with_unsaved_work();
        app.autosave_file = Some(directory.join("autosave.hani"));
        app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
        app.write_autosave();
        assert!(app.has_autosave(), "the fixture needs a crash net to protect");

        // Where `save_project` will put its temporary, pointed somewhere that
        // accepts the write and gives nothing back.
        let temporary = path.with_extension("hani.tmp");
        std::os::unix::fs::symlink("/dev/null", &temporary).unwrap();

        app.save_project(&path);

        assert!(
            app.status.contains("could not write"),
            "a file that read back empty reported: {}",
            app.status
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"the previous save, which must survive",
            "an unverifiable save replaced the file it could not prove it had written"
        );
        assert!(app.unsaved, "an unverifiable save cleared the unsaved marker");
        assert!(app.project_path.is_none(), "an unverifiable save adopted the path anyway");
        assert!(
            app.has_autosave(),
            "an unverifiable save deleted the crash net, which is the whole failure the \
             verification exists to stop"
        );
        assert!(
            std::fs::symlink_metadata(&temporary).is_err(),
            "the temporary was left on disk after the verification refused it"
        );
        assert!(
            std::path::Path::new("/dev/null").exists(),
            "the cleanup followed the symlink instead of unlinking it"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    /// The last failure arm, and the only one reached with a good file in hand:
    /// the write worked, the read-back agreed, and the rename still failed.
    ///
    /// It is here for two reasons. It is the one arm that runs *after* the
    /// verification, so reaching it at all proves execution got past that gate
    /// rather than around it. And it is the arm where the temporary holds the
    /// user's only new copy, so leaving it behind, or clearing `unsaved`
    /// because the bytes did reach a disk somewhere, would both be wrong.
    ///
    /// The failure is forced by putting a DIRECTORY at the TARGET, which makes
    /// `rename` fail with `EISDIR` while leaving every step before it -- create,
    /// write, verify -- entirely successful.
    #[test]
    fn a_save_that_cannot_replace_the_target_keeps_the_crash_net() {
        let directory = scratch("save-rename");
        let path = directory.join("sculpt.hani");
        std::fs::create_dir_all(&path).unwrap();

        let mut app = app_with_unsaved_work();
        app.autosave_file = Some(directory.join("autosave.hani"));
        app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
        app.write_autosave();
        assert!(app.has_autosave(), "the fixture needs a crash net to protect");

        app.save_project(&path);

        assert!(
            app.status.contains("could not replace"),
            "a failed rename reported: {}",
            app.status
        );
        assert!(app.unsaved, "a failed rename cleared the unsaved marker");
        assert!(app.project_path.is_none(), "a failed rename adopted the path anyway");
        assert!(app.has_autosave(), "a failed rename deleted the crash net");
        assert!(
            !path.with_extension("hani.tmp").exists(),
            "a failed rename left the temporary behind, where the user would find a file \
             holding work the application says is unsaved"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    ///
    /// One shared number made Move look broken: for Move, strength is the
    /// fraction of the drag the surface follows, so Draw's 0.15 meant the form
    /// crawled at a seventh of the pointer.
    #[test]
    fn each_brush_remembers_its_own_strength() {
        let mut app = app();
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        assert!((app.brush.strength - 0.15).abs() < 1.0e-6, "draw: {}", app.brush.strength);

        update(&mut app, Message::BrushKindChanged(BrushKind::Move));
        assert!(
            app.brush.strength > 0.9,
            "a grab tool at {} would follow a seventh of the drag",
            app.brush.strength
        );

        // A deliberate change sticks, and survives a round trip through another
        // brush.
        update(&mut app, Message::BrushStrengthChanged(0.5));
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        assert!((app.brush.strength - 0.15).abs() < 1.0e-6, "draw was overwritten");
        update(&mut app, Message::BrushKindChanged(BrushKind::Move));
        assert!((app.brush.strength - 0.5).abs() < 1.0e-6, "move forgot its setting");
    }

    /// A Move drag must follow the pointer across the SCREEN, not crawl around
    /// the form.
    ///
    /// Raycasting the pointer onto the surface gives a target that slides along
    /// the curve, so the vector from the grab point stays short and keeps
    /// turning. Dragging past the silhouette is the case that separates the two:
    /// there is no surface under the cursor at all there, and a raycast version
    /// simply stops.
    #[test]
    fn a_move_drag_keeps_working_past_the_edge_of_the_model() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        update(&mut app, Message::BrushKindChanged(BrushKind::Move));
        update(&mut app, Message::BrushRadiusChanged(10.0));

        let probe = Vec3::new(0.0, 0.0, 30.0);
        let before = app.doc.active_volume().sample_world(probe);

        press(&mut app, centre_of_viewport());
        // Well past the silhouette of the ball.
        for step in 1..=40 {
            app.on_pointer(PointerEvent::Moved {
                position: centre_of_viewport() + Vector::new(step as f32 * 12.0, 0.0),
                size: SIZE,
            });
        }
        release(&mut app);

        assert_ne!(
            app.doc.active_volume().sample_world(probe),
            before,
            "the drag stopped at the silhouette, so it is still raycasting the surface"
        );
        assert!(app.history_stats.undo_entries > 0, "a long drag recorded no undo entry");
    }

    /// Move through the APPLICATION, not through `Brush::apply`.
    ///
    /// Every existing Move test builds a `Brush` and a `Stamp` by hand, which
    /// tests the arithmetic and says nothing about whether a drag in the
    /// viewport ever reaches it.
    #[test]
    fn dragging_with_move_selected_actually_changes_the_model() {
        let mut app = app();
        update(&mut app, Message::BrushKindChanged(BrushKind::Move));
        update(&mut app, Message::BrushStrengthChanged(1.0));
        update(&mut app, Message::BrushRadiusChanged(10.0));

        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        let probe = app
            .surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0))
            .expect("the centre hits the ball");
        let before = app.doc.active_volume().sample_world(probe);

        press(&mut app, centre_of_viewport());
        for step in 1..=12 {
            app.on_pointer(PointerEvent::Moved {
                position: centre_of_viewport() + Vector::new(step as f32 * 4.0, 0.0),
                size: SIZE,
            });
        }
        release(&mut app);

        assert!(
            app.history_stats.undo_entries > 0,
            "a Move drag recorded no undo entry, so it did nothing at all"
        );
        assert_ne!(
            app.doc.active_volume().sample_world(probe),
            before,
            "a Move drag across the model left the field untouched"
        );
    }

    /// Reach and cost together, since Move trades one against the other.
    ///
    /// Printed rather than asserted: these are the numbers the reach cap was
    /// chosen against, and the point is that a future change can re-run them.
    /// `the_surface_follows_the_pointer_through_the_application` is the one
    /// that fails if the reach collapses again.
    #[test]
    fn measure_move_reach_and_cost() {
        for (radius, strength) in
            [(3.0f32, 0.15f32), (3.0, 1.0), (10.0, 0.15), (10.0, 1.0), (20.0, 0.15), (20.0, 1.0)]
        {
            let mut app = app();
            aim_at_the_front(&mut app);
            update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
            update(&mut app, Message::BrushRadiusChanged(radius.min(6.0)));
            update(&mut app, Message::BrushStrengthChanged(0.8));
            for _ in 0..6 {
                press(&mut app, centre_of_viewport());
                release(&mut app);
            }

            update(&mut app, Message::BrushKindChanged(BrushKind::Move));
            update(&mut app, Message::BrushRadiusChanged(radius));
            update(&mut app, Message::BrushStrengthChanged(strength));

            let before = bump_x(&app);
            app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
            let from = app.surface_under(pixel(centre_of_viewport())).expect("the centre hits");

            press(&mut app, centre_of_viewport());
            let mut worst = 0.0f32;
            let mut total_ms = 0.0f32;
            let mut to = from;
            for step in 1..=30 {
                let at = centre_of_viewport() + Vector::new(step as f32 * 5.0, 0.0);
                app.on_pointer(PointerEvent::Moved { position: at, size: SIZE });
                worst = worst.max(app.perf.edit_ms);
                total_ms += app.perf.edit_ms;
                if let Some(point) = app.surface_under(pixel(at)) {
                    to = point;
                }
            }
            release(&mut app);

            let moved = bump_x(&app) - before;
            eprintln!(
                "radius {radius:>5.1} strength {strength:>4.2}: pointer travelled \
                 {:>5.1} mm, surface followed {moved:>+6.2} mm (cap {:>5.2} mm), \
                 worst event {worst:>6.2} ms, whole drag {total_ms:>7.2} ms",
                from.distance(to),
                Brush { kind: BrushKind::Move, radius, ..app.brush }.max_drag(),
            );
        }
    }

    /// Where the bump raised at the front of the ball sits along X, weighted by
    /// how much material is standing proud of the sphere at each slice.
    ///
    /// A drag has to carry this along with it, and it is measured rather than
    /// assumed to have stayed put -- which is the whole difference between
    /// "something changed" and "the surface followed the pointer".
    fn bump_x(app: &Hani) -> f32 {
        let mut weighted = 0.0;
        let mut total = 0.0;
        for step in -120..=120 {
            let x = step as f32 * 0.25;
            // Where the surface crosses along +Z at this slice.
            let mut z = 60.0f32;
            while z > 0.0 {
                if app.doc.active_volume().sample_world(Vec3::new(x, 0.0, z)) < 0.0 {
                    break;
                }
                z -= 0.05;
            }
            let raised = (z - (MODEL_RADIUS_MM * MODEL_RADIUS_MM - x * x).max(0.0).sqrt()).max(0.0);
            weighted += x * raised;
            total += raised;
        }
        assert!(total > 0.0, "there is no bump to measure");
        weighted / total
    }

    fn pixel(at: Vector) -> Vec2 {
        Vec2::new(at.x, at.y)
    }

    /// Look straight down -Z, so a horizontal drag on screen is a drag along
    /// world X and the measurements above are in a frame anyone can check.
    fn aim_at_the_front(app: &mut Hani) {
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
    }

    /// The reach the old incremental Move could not deliver.
    ///
    /// It moved the surface **0.02 mm** for a full viewport drag at the default
    /// radius, which is why it was reported as doing nothing at all. This
    /// asserts a real magnitude rather than "something changed", because
    /// "something changed" is exactly what the broken version also did.
    #[test]
    fn the_surface_follows_the_pointer_through_the_application() {
        let mut app = app();
        aim_at_the_front(&mut app);
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        update(&mut app, Message::BrushRadiusChanged(6.0));
        update(&mut app, Message::BrushStrengthChanged(0.8));
        for _ in 0..6 {
            press(&mut app, centre_of_viewport());
            release(&mut app);
        }

        update(&mut app, Message::BrushKindChanged(BrushKind::Move));
        update(&mut app, Message::BrushRadiusChanged(10.0));
        update(&mut app, Message::BrushStrengthChanged(1.0));

        let before = bump_x(&app);
        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        let from = app.surface_under(pixel(centre_of_viewport())).expect("the centre hits");

        press(&mut app, centre_of_viewport());
        let mut to = from;
        for step in 1..=10 {
            let at = centre_of_viewport() + Vector::new(step as f32 * 5.0, 0.0);
            app.on_pointer(PointerEvent::Moved { position: at, size: SIZE });
            if let Some(point) = app.surface_under(pixel(at)) {
                to = point;
            }
        }
        release(&mut app);

        let pointer = from.distance(to);
        let moved = bump_x(&app) - before;
        assert!(pointer > 3.0, "the test did not drag far enough to mean anything: {pointer} mm");
        assert!(
            moved > pointer * 0.4,
            "a {pointer:.1} mm drag moved the surface {moved:.2} mm, which is the order of \
             magnitude the incremental version failed at"
        );
    }

    /// The property locking the field buys, seen from the application.
    #[test]
    fn a_drag_out_and_back_through_the_application_returns_the_form() {
        let mut app = app();
        aim_at_the_front(&mut app);
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        update(&mut app, Message::BrushRadiusChanged(6.0));
        update(&mut app, Message::BrushStrengthChanged(0.8));
        for _ in 0..6 {
            press(&mut app, centre_of_viewport());
            release(&mut app);
        }

        let probes: Vec<Vec3> = (-40..=40)
            .map(|step| Vec3::new(step as f32 * 0.5, 0.0, MODEL_RADIUS_MM - 2.0))
            .collect();
        let before: Vec<f32> =
            probes.iter().map(|p| app.doc.active_volume().sample_world(*p)).collect();

        update(&mut app, Message::BrushKindChanged(BrushKind::Move));
        update(&mut app, Message::BrushRadiusChanged(10.0));
        update(&mut app, Message::BrushStrengthChanged(1.0));

        press(&mut app, centre_of_viewport());
        let mut path: Vec<f32> = (1..=8).map(|step| step as f32 * 4.0).collect();
        path.extend((0..=8).rev().map(|step| step as f32 * 4.0));
        for offset in path {
            app.on_pointer(PointerEvent::Moved {
                position: centre_of_viewport() + Vector::new(offset, 0.0),
                size: SIZE,
            });
        }
        release(&mut app);

        let worst = probes
            .iter()
            .zip(&before)
            .map(|(probe, was)| (app.doc.active_volume().sample_world(*probe) - was).abs())
            .fold(0.0f32, f32::max);
        // Looser than the core test's thousandth, and for two reasons that are
        // the application's rather than the algorithm's. The world point comes
        // from a raycast against the surface as it currently stands, so coming
        // back to the same pixel is not quite coming back to the same point;
        // and a drag that has shrunk to within a quarter voxel of where it
        // already was is skipped rather than redone. Both are bounded by a
        // fraction of a voxel, which is what this asserts -- the values are in
        // voxels, so a twentieth here is a hundredth of a voxel.
        assert!(worst < 0.05, "a drag out and back left the surface {worst} different");
    }

    // --- the plane cut -------------------------------------------------------

    /// Which side of the dragged line is removed, pinned by observation rather
    /// than by reasoning about cross products on paper.
    ///
    /// This is the one thing about the cut that cannot be got right by thinking
    /// about it: the sign depends on the ray order, the handedness of the
    /// camera basis, and whether the pixel-to-NDC step flips Y. Get it backwards
    /// and the tool takes the half the user meant to keep -- which is
    /// destructive, and only obvious after the fact.
    /// **Alt turns the cut into a parting**: the same line, and the body
    /// becomes two that register on each other instead of losing a side.
    ///
    /// Driven through the pointer and not by calling the engine, because the
    /// engine has its own tests and what can go wrong HERE is the wiring: the
    /// modifier read at the wrong moment, the source's slots not released,
    /// the tool left armed, the history given two entries or none.
    #[test]
    fn alt_parts_the_body_in_two_with_keys_and_one_undo_puts_it_back() {
        let mut app = app();
        // Parting is Print's cut (`docs/WORKSPACES.md`); in Sculpt a cut
        // only cuts.
        app.desk.now = crate::message::Workspace::Print;
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        let source = app.doc.active();
        let before = app.doc.active_volume().stats();
        let undo_before = app.history.stats().undo_entries;

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        app.alt = true;
        assert_eq!(
            app.live_tool_label(),
            "PART — KEEP BOTH",
            "the card does not say what alt does"
        );
        let middle_y = SIZE.y / 2.0;
        press(&mut app, Vector::new(SIZE.x * 0.1, middle_y));
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.9, middle_y),
            size: SIZE,
        });
        release(&mut app);
        app.alt = false;

        assert_ne!(app.tool, Tool::Cut, "the tool stayed armed after parting");
        assert!(app.status.starts_with("parted "), "{}", app.status);
        assert!(app.status.contains("key"), "the status line says nothing of keys: {}", app.status);
        assert!(app.unsaved, "a parting did not mark the document unsaved");
        assert_eq!(app.doc.body_count(), 2, "the body did not become two");
        assert!(app.doc.volume(source).is_none(), "the source is still in the document");
        assert_eq!(
            app.history.stats().undo_entries,
            undo_before + 1,
            "a parting should be exactly one undo entry"
        );

        // Both halves are real, printable, and on either side of the line.
        let mut visible = Vec::new();
        app.doc.saved_visibility(&mut visible);
        let bodies = app.doc.export_bodies(&visible);
        assert_eq!(bodies.len(), 2);
        for (meta, _, report) in &bodies {
            assert!(report.is_printable(), "{}: {}", meta.name, report.summary());
        }
        let above = Vec3::new(0.0, 12.0, 0.0);
        let below = Vec3::new(0.0, -12.0, 0.0);
        let holds = |at: Vec3| -> Vec<bool> {
            app.doc
                .nodes()
                .iter()
                .filter_map(|node| node.volume())
                .map(|volume| volume.sample_world(at) < 0.0)
                .collect()
        };
        assert_eq!(holds(above).iter().filter(|held| **held).count(), 1, "one part holds the top");
        assert_eq!(
            holds(below).iter().filter(|held| **held).count(),
            1,
            "one part holds the bottom"
        );
        assert_ne!(holds(above), holds(below), "one part holds both the top and the bottom");

        // One undo, and the body that was there is there again.
        update(&mut app, Message::Undo);
        assert_eq!(
            app.doc.body_count(),
            1,
            "undoing a parting left {} bodies",
            app.doc.body_count()
        );
        let after = app.doc.volume(source).expect("the source is back under its own id").stats();
        assert_eq!(after.dense_bricks, before.dense_bricks, "the body came back a different size");
    }

    /// The key settings reach the parting. Without this the controls could
    /// move a number on the panel and change nothing that gets cut.
    #[test]
    fn the_key_settings_decide_what_a_parting_cuts() {
        use crate::message::KeySetting;
        use hani_core::part::KeyKind;

        let part = |app: &mut Hani| {
            // Parting is Print's cut (`docs/WORKSPACES.md`).
            app.desk.now = crate::message::Workspace::Print;
            app.camera.yaw = 0.0;
            app.camera.pitch = 0.0;
            app.publish_camera();
            update(app, Message::ToolChanged(Tool::Cut));
            // The line cut, not the ring. See `app/ring_cut.rs`.
            update(app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
            app.alt = true;
            let middle_y = SIZE.y / 2.0;
            press(app, Vector::new(SIZE.x * 0.1, middle_y));
            app.on_pointer(PointerEvent::Moved {
                position: Vector::new(SIZE.x * 0.9, middle_y),
                size: SIZE,
            });
            release(app);
            app.alt = false;
            assert!(app.status.starts_with("parted "), "{}", app.status);
        };
        // Solid volume of every body together, by counting voxels that read
        // inside.
        let solid = |app: &Hani| -> usize {
            app.doc
                .nodes()
                .iter()
                .filter_map(|node| node.volume())
                .map(|volume| {
                    let (lo, hi) = volume.world_bounds().expect("a body has bricks");
                    let (lo, hi) = volume.voxel_bounds(lo, hi);
                    let mut inside = 0;
                    for z in lo.z..=hi.z {
                        for y in lo.y..=hi.y {
                            for x in lo.x..=hi.x {
                                inside += usize::from(
                                    volume.sample_voxel(glam::IVec3::new(x, y, z)) < 0.0,
                                );
                            }
                        }
                    }
                    inside
                })
                .sum()
        };

        // A peg adds to one part what it takes from the other, less the
        // clearance. A pin hole only takes, from both.
        let mut pegged = app();
        part(&mut pegged);
        let mut pinned = app();
        update(&mut pinned, Message::KeySettingChanged(KeySetting::Kind(KeyKind::PinHole)));
        assert_eq!(pinned.key_spec.kind, KeyKind::PinHole, "the setting did not arrive");
        part(&mut pinned);
        assert!(
            solid(&pinned) + 50 < solid(&pegged),
            "pin holes left {} voxels solid and pegs {}: the kind did not reach the cut",
            solid(&pinned),
            solid(&pegged)
        );

        // And a setting from outside the sliders' range is clamped, not
        // passed on.
        update(&mut pinned, Message::KeySettingChanged(KeySetting::Radius(-3.0)));
        update(&mut pinned, Message::KeySettingChanged(KeySetting::Clearance(40.0)));
        assert_eq!(pinned.key_spec.radius_mm, 0.5);
        assert_eq!(pinned.key_spec.clearance_mm, 0.6);
    }

    /// A parting is one plane. A lasso with alt held parts nothing and says
    /// why, where quietly cutting instead would take material the user asked
    /// to keep.
    #[test]
    fn alt_over_a_lasso_parts_nothing_and_cuts_nothing() {
        let mut app = app();
        // Parting is Print's cut (`docs/WORKSPACES.md`); in Sculpt a cut
        // only cuts.
        app.desk.now = crate::message::Workspace::Print;
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        let before = app.doc.active_volume().stats();

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        app.alt = true;
        // A closed loop round the middle of the view.
        let (cx, cy) = (SIZE.x / 2.0, SIZE.y / 2.0);
        press(&mut app, Vector::new(cx + 80.0, cy));
        for step in 1..=24 {
            let angle = step as f32 / 24.0 * std::f32::consts::TAU;
            app.on_pointer(PointerEvent::Moved {
                position: Vector::new(cx + 80.0 * angle.cos(), cy + 80.0 * angle.sin()),
                size: SIZE,
            });
        }
        release(&mut app);
        app.alt = false;

        assert!(app.status.starts_with("not parted"), "{}", app.status);
        assert_eq!(app.doc.body_count(), 1);
        let after = app.doc.active_volume().stats();
        assert_eq!(after.dense_bricks, before.dense_bricks, "the lasso cut the body after all");
        assert_eq!(app.history.stats().undo_entries, 0, "a refused parting reached the history");
    }

    #[test]
    fn a_left_to_right_drag_removes_a_consistent_side() {
        let mut app = app();
        // A known camera: looking straight down -Z at the origin.
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();

        let middle_y = SIZE.y / 2.0;
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        assert_eq!(app.tool, Tool::Cut, "the cut did not arm");

        // Drag left to right across the middle of the viewport.
        press(&mut app, Vector::new(SIZE.x * 0.1, middle_y));
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.9, middle_y),
            size: SIZE,
        });
        release(&mut app);

        assert_ne!(app.tool, Tool::Cut, "the cut stayed armed after being used");
        assert!(app.unsaved, "a cut did not mark the document unsaved");

        let above = app.doc.active_volume().sample_world(Vec3::new(0.0, 12.0, 0.0));
        let below = app.doc.active_volume().sample_world(Vec3::new(0.0, -12.0, 0.0));
        assert_ne!(
            above < 0.0,
            below < 0.0,
            "the cut removed both halves or neither: above {above}, below {below}"
        );
        // Record which one it actually is, so a change of sign fails here
        // rather than in someone's sculpt. If this assertion is what fails
        // after a camera change, check the ray order in `finish_cut` before
        // editing the expectation.
        assert!(
            below < 0.0 && above > 0.0,
            "a left to right drag should keep the LOWER half on screen: \
             above {above}, below {below}"
        );
    }

    /// The pixel column whose view ray crosses the z = 0 plane at world `x`,
    /// for a camera looking straight down -Z. A vertical drag there is a plane
    /// that stands at that x where the model's equator is.
    fn column_at_world_x(app: &Hani, x: f32) -> f32 {
        let x_at = |px: f32| {
            let (origin, ray) = app.ray_through(Vec2::new(px, SIZE.y / 2.0));
            origin.x + ray.x * (-origin.z / ray.z)
        };
        let (mut low, mut high) = (0.0_f32, SIZE.x);
        for _ in 0..40 {
            let middle = (low + high) / 2.0;
            if x_at(middle) < x {
                low = middle;
            } else {
                high = middle;
            }
        }
        (low + high) / 2.0
    }

    /// **The report this exists for**: with X mirroring on, a plane cut down
    /// one side of the model took that side and left the other -- while every
    /// stroke that session had landed on both. Now the cut takes both, as one
    /// gesture and one undo entry.
    #[test]
    fn a_cut_with_x_mirror_on_takes_both_sides() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        assert!(app.symmetry_settings().axis(MirrorAxis::X), "X did not turn on");

        // A plane standing at x = 10 on the equator, dragged top to bottom: the
        // left of the arrow is screen-right, so everything past x = 10 goes.
        let column = column_at_world_x(&app, 10.0);
        let sample =
            |app: &Hani, x: f32| app.doc.active_volume().sample_world(Vec3::new(x, 0.0, 0.0));
        assert!(sample(&app, 14.0) < 0.0 && sample(&app, -14.0) < 0.0, "the fixture is not a ball");
        let entries_before = app.history.stats().undo_entries;

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        press(&mut app, Vector::new(column, SIZE.y * 0.1));
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(column, SIZE.y * 0.9),
            size: SIZE,
        });
        release(&mut app);

        assert!(sample(&app, 14.0) > 0.0, "the drawn side stayed: {}", app.status);
        assert!(sample(&app, -14.0) > 0.0, "the mirrored side stayed: {}", app.status);
        assert!(sample(&app, 0.0) < 0.0, "the middle went too: {}", app.status);
        assert_eq!(
            app.history.stats().undo_entries,
            entries_before + 1,
            "a mirrored cut is one gesture and one entry"
        );
        assert!(app.status.contains("mirrored X"), "the status did not say so: {}", app.status);
    }

    /// And with the mirror off, the same drag takes one side only -- the cut
    /// that has always shipped is untouched.
    #[test]
    fn a_cut_with_the_mirror_off_takes_one_side() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
        assert!(app.symmetry_settings().is_off());

        let column = column_at_world_x(&app, 10.0);
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        press(&mut app, Vector::new(column, SIZE.y * 0.1));
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(column, SIZE.y * 0.9),
            size: SIZE,
        });
        release(&mut app);

        let sample = |x: f32| app.doc.active_volume().sample_world(Vec3::new(x, 0.0, 0.0));
        assert!(sample(14.0) > 0.0, "the drawn side stayed: {}", app.status);
        assert!(sample(-14.0) < 0.0, "the other side went with no mirror on: {}", app.status);
        assert!(!app.status.contains("mirrored"), "{}", app.status);
    }

    /// What is shown is what goes, on both sides: with a mirror on, the
    /// preview draws the twin as well as the outline that was drawn.
    #[test]
    fn the_cut_preview_draws_the_twin_when_a_mirror_is_on() {
        let mut app = app();
        // Looking straight down -Z, so world x and screen x agree and the
        // side assertion below means what it says.
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        press(&mut app, Vector::new(SIZE.x * 0.65, SIZE.y * 0.2));
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.65, SIZE.y * 0.8),
            size: SIZE,
        });

        let mut alone = hani_gpu::OverlayBatch::default();
        app.add_cut_preview(&mut alone);
        assert!(!alone.lines.is_empty(), "no preview at all");

        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        assert!(app.symmetry_settings().axis(MirrorAxis::X), "X did not turn on");
        let mut mirrored = hani_gpu::OverlayBatch::default();
        app.add_cut_preview(&mut mirrored);
        assert_eq!(mirrored.lines.len(), alone.lines.len() * 2, "the twin was not drawn");
        assert_eq!(
            mirrored.surfaces.len(),
            alone.surfaces.len() * 2,
            "the twin's side was not shaded"
        );

        // The twin sits on the other side of the plane. The drag was to the
        // right of centre, so the outline is at positive x and its twin, once
        // reflected and brought back onto the preview plane, at negative x.
        let mean_x = |vertices: &[hani_gpu::OverlayVertex]| {
            vertices.iter().map(|vertex| vertex.position[0]).sum::<f32>() / vertices.len() as f32
        };
        assert!(mean_x(&alone.lines) > 0.0, "the fixture's outline is not at positive x");
        assert!(
            mean_x(&mirrored.lines[alone.lines.len()..]) < 0.0,
            "the twin is not across the plane"
        );
    }

    /// A cut is destructive, so a click must never be one.
    #[test]
    fn a_click_with_the_cut_armed_does_nothing() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        let before = app.doc.active_volume().brick_coords().count();

        press(&mut app, centre_of_viewport());
        release(&mut app);

        assert_eq!(app.doc.active_volume().brick_coords().count(), before, "a click cut the model");
        assert!(!app.unsaved, "a click that cut nothing marked the document unsaved");
        assert!(app.status.contains("cancelled"), "reported: {}", app.status);
    }

    /// Add a sphere to whatever is already there.
    ///
    /// **Not `seed_sphere`, which OVERWRITES.** Seeding clears every brick the
    /// sphere's box touches before writing, so seeding a small sphere onto a
    /// large one carves a moat around it rather than adding a lump to it: the
    /// fixture ends up as a ball with a pit, and a test that asserts "the spur
    /// went" passes for the wrong reason. A union is `min` of the two distance
    /// fields, which is what this does.
    fn union_sphere(volume: &mut hani_core::Volume, centre: Vec3, radius: f32) {
        let voxel_size = volume.voxel_size();
        let band = hani_core::NARROW_BAND * voxel_size;
        let (lo, hi) =
            volume.voxel_bounds(centre - (radius + band * 2.0), centre + (radius + band * 2.0));
        volume.edit_voxels(lo, hi, |_, position, value| {
            let outside = (position.distance(centre) - radius) / voxel_size;
            value.min(outside).clamp(hani_core::INSIDE, hani_core::OUTSIDE)
        });
    }

    /// Drag a closed loop, from a list of viewport positions.
    fn drag_loop(app: &mut Hani, points: &[Vector]) {
        let first = *points.first().expect("a loop needs points");
        press(app, first);
        for point in points.iter().skip(1) {
            app.on_pointer(PointerEvent::Moved { position: *point, size: SIZE });
        }
        app.on_pointer(PointerEvent::Moved { position: first, size: SIZE });
        release(app);
    }

    /// A circle of viewport positions around the middle of the screen.
    fn loop_around(radius: f32, steps: usize) -> Vec<Vector> {
        let centre = centre_of_viewport();
        (0..steps)
            .map(|step| {
                let angle = step as f32 / steps as f32 * std::f32::consts::TAU;
                centre + Vector::new(angle.cos() * radius, angle.sin() * radius)
            })
            .collect()
    }

    /// **The region cut is the region drawn**, checked in world space rather
    /// than by looking at the screen.
    ///
    /// This is the test whose absence let the apex bug survive a design review.
    /// The cut used the ray's NEAR-PLANE point as the pyramid's apex instead of
    /// the eye; with one plane that cancels exactly, and with several it
    /// displaces every side by a different amount. It was invisible to any test
    /// comparing the preview against the drawing, because the preview projects
    /// the same screen points and is therefore screen-exact either way.
    ///
    /// So this projects the RESULT back: every point that lost material must
    /// fall inside the drawn polygon, and points well outside it must be
    /// untouched. A displaced apex fails the second half.
    #[test]
    fn the_region_cut_matches_the_region_drawn() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));

        let radius = SIZE.y * 0.15;
        drag_loop(&mut app, &loop_around(radius, 24));
        assert!(app.unsaved, "the lasso did not cut: {}", app.status);

        // Probe a ring of world points at the depth the camera focuses on,
        // spanning well inside the loop and well outside it.
        let centre = centre_of_viewport();
        let volume = app.doc.active_volume();
        let mut inside_survived = 0;
        let mut outside_removed = 0;
        for step in 0..48 {
            let angle = step as f32 / 48.0 * std::f32::consts::TAU;
            for (scale, expect_gone) in [(0.45_f32, true), (2.2_f32, false)] {
                let pixel = centre + Vector::new(angle.cos(), angle.sin()) * (radius * scale);
                // Where that pixel meets the sphere's front face, before the
                // cut: the model is a ball of known radius at the origin.
                let (origin, ray) = app.ray_through(Vec2::new(pixel.x, pixel.y));
                let Some(hit) = ray_meets_ball(origin, ray, MODEL_RADIUS_MM) else {
                    continue;
                };
                // A little inside the surface, so a grazing sample does not
                // decide the outcome.
                let probe = hit + ray * 0.5;
                let gone = volume.sample_world(probe) > 0.0;
                if expect_gone && !gone {
                    inside_survived += 1;
                }
                if !expect_gone && gone {
                    outside_removed += 1;
                }
            }
        }
        assert_eq!(inside_survived, 0, "material survived well inside the drawn loop");
        assert_eq!(outside_removed, 0, "material was removed well outside the drawn loop");
    }

    /// Where a ray first meets a ball at the origin, if it does.
    fn ray_meets_ball(origin: Vec3, ray: Vec3, radius: f32) -> Option<Vec3> {
        let b = origin.dot(ray);
        let c = origin.length_squared() - radius * radius;
        let discriminant = b * b - c;
        if discriminant < 0.0 {
            return None;
        }
        let t = -b - discriminant.sqrt();
        (t > 0.0).then(|| origin + ray * t)
    }

    /// **The preview must be drawn in FRONT of the model it describes.**
    ///
    /// This is the one that `cargo test` could not see and a screenshot could.
    /// The outline was first projected onto the plane through the camera's
    /// target -- which is the middle of the model -- so the polygon sat INSIDE
    /// the sphere and the depth test threw it away. Every headless assertion
    /// passed: the geometry was built, the vertex counts were right, the shape
    /// was correct, and the screen showed nothing at all.
    ///
    /// The straight-line cut hid it for a while, because its band reaches three
    /// model radii out and the part past the silhouette has nothing in front of
    /// it. So the band appeared over the background and vanished over the
    /// model, which reads as a rendering quirk rather than as a preview drawn
    /// in the wrong place.
    ///
    /// Depth is what is asserted, not pixels, because depth is the thing that
    /// was wrong and it is checkable without a compositor.
    #[test]
    fn the_live_preview_is_drawn_in_front_of_the_model() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));

        let points = loop_around(SIZE.y * 0.15, 12);
        press(&mut app, points[0]);
        for point in points.iter().skip(1) {
            app.on_pointer(PointerEvent::Moved { position: *point, size: SIZE });
        }
        app.on_pointer(PointerEvent::Moved { position: points[0], size: SIZE });

        assert!(!app.overlay.lines.is_empty(), "the live lasso drew no outline");

        // The nearest the model can be to the eye: the camera looks at the
        // origin and the body is a ball of MODEL_RADIUS_MM about it.
        let eye = app.camera.eye();
        let nearest_surface = eye.length() - MODEL_RADIUS_MM;
        for vertex in app.overlay.lines.iter().chain(&app.overlay.surfaces) {
            let away = Vec3::from_array(vertex.position).distance(eye);
            assert!(
                away < nearest_surface,
                "a preview vertex sits {away} from the eye, behind the model's near face at \
                 {nearest_surface} -- the depth test will hide it"
            );
        }
    }

    /// **The depth cap spares the wall behind the spur.**
    ///
    /// The fixture is the one the design argues about: a lump sticking out with
    /// another surface close behind it, close enough that placing the cap one
    /// band past the lump's back face would eat into the wall. Circling the
    /// lump must take the lump and leave the wall EXACTLY as it was.
    ///
    /// `assert_same_field` rather than a thickness measurement, because a
    /// thickness measurement is what `is_printable` does not have: it counts
    /// holes and winding, so a wall thinned from the front with a clean step is
    /// still "printable" and no other assertion in this repo would notice.
    #[test]
    fn a_lasso_over_a_spur_leaves_the_wall_behind_it_alone() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();

        // Rebuilt as a spur in front of a wall, in camera-relative terms so the
        // fixture does not depend on which way yaw zero points.
        let eye = app.camera.eye();
        let toward_camera = (eye - app.camera.target).normalize();
        {
            let volume = app.doc.active_volume_mut();
            *volume = hani_core::Volume::new(volume.voxel_size());
            // The wall: a big ball centred behind the origin, so its near face
            // sits a little way behind the spur.
            volume.seed_sphere(-toward_camera * 34.0, 30.0);
            // The spur: small, in front, straddling the wall's near face so it
            // is attached rather than floating. UNIONED, because seeding it
            // would clear the wall's bricks around it and leave a moat.
            union_sphere(volume, toward_camera * 6.0, 7.0);
            volume.mark_everything_dirty();
        }
        app.rebuild_everything();

        let before = app.doc.active_volume().duplicated(glam::IVec3::ZERO);

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        drag_loop(&mut app, &loop_around(SIZE.y * 0.06, 20));
        assert!(app.unsaved, "the lasso did not cut: {}", app.status);

        // The wall's far side, well behind anything the spur occupies: it must
        // read exactly as it did.
        let deep = -toward_camera * 50.0;
        let after = app.doc.active_volume().sample_world(deep);
        assert_eq!(
            before.sample_world(deep),
            after,
            "the cut reached through to the back of the wall"
        );
        // And the spur itself is gone.
        let spur = toward_camera * 10.0;
        assert!(
            app.doc.active_volume().sample_world(spur) > 0.0,
            "the spur survived the lasso that circled it"
        );
    }

    /// Shift asks for the old behaviour, and it must actually get it: the same
    /// loop with shift held goes all the way through.
    #[test]
    fn shift_makes_the_same_lasso_cut_through() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        let eye = app.camera.eye();
        let toward_camera = (eye - app.camera.target).normalize();
        {
            let volume = app.doc.active_volume_mut();
            *volume = hani_core::Volume::new(volume.voxel_size());
            volume.seed_sphere(-toward_camera * 34.0, 30.0);
            union_sphere(volume, toward_camera * 6.0, 7.0);
            volume.mark_everything_dirty();
        }
        app.rebuild_everything();

        app.shift = true;
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        drag_loop(&mut app, &loop_around(SIZE.y * 0.06, 20));

        let deep = -toward_camera * 50.0;
        assert!(
            app.doc.active_volume().sample_world(deep) > 0.0,
            "shift did not cut through: {}",
            app.status
        );
    }

    /// **The hole is the same size where it goes in and where it comes out.**
    ///
    /// The cutter is built along the view direction rather than from the eye,
    /// so its sides are parallel. Built from the eye instead -- which is the
    /// obvious construction and the one this shipped with first -- the removed
    /// region is the view frustum's own wedge and widens exactly as the
    /// projection does. At the framing this application picks that is not
    /// subtle: the eye sits about 90 mm from a 30 mm ball, so the front face is
    /// 60 mm away and the back 120, and **the exit hole comes out twice the
    /// diameter of the entry hole**.
    ///
    /// Measured rather than reasoned about, because the factor depends on the
    /// framing rule, the field of view and the model's size all at once, and a
    /// test that recomputed those would be asserting its own arithmetic.
    #[test]
    fn a_cut_through_leaves_the_same_hole_at_both_ends() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();

        let eye = app.camera.eye();
        let facing = (app.camera.target - eye).normalize();
        // Any direction across the view, to scan outward along.
        let across = facing.cross(Vec3::Y).normalize_or(Vec3::X);

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        // Shift, so the cut goes all the way through and there are two ends to
        // compare. With the depth cap on there is only one.
        app.shift = true;
        drag_loop(&mut app, &loop_around(SIZE.y * 0.10, 24));
        assert!(app.unsaved, "the lasso did not cut: {}", app.status);

        // How far the hole reaches from the axis, on a slab a little inside
        // each face so a grazing sample cannot decide it.
        let reach_at = |depth: f32| {
            let volume = app.doc.active_volume();
            let mut last_gone = 0.0_f32;
            let mut radius = 0.0_f32;
            while radius < 28.0 {
                let probe = facing * depth + across * radius;
                if volume.sample_world(probe) > 0.0 {
                    last_gone = radius;
                } else {
                    break;
                }
                radius += 0.25;
            }
            last_gone
        };

        let front = reach_at(-25.0);
        let back = reach_at(25.0);
        assert!(front > 1.0, "no hole at the front face: {front}");
        assert!(back > 1.0, "no hole at the back face: {back}");
        assert!(
            (back - front).abs() < front * 0.15,
            "the hole flares through the model: {front:.2} mm going in, {back:.2} mm coming \
             out -- the cutter is a pyramid from the eye rather than a prism"
        );
    }

    /// **Ctrl with a loop selects instead of cutting, and the selection becomes
    /// a body.**
    ///
    /// This is the whole "circle the lump and it becomes a layer" route, end to
    /// end through the application: the same drag, the same hull, the same
    /// classification, writing protection instead of removing material -- and
    /// then the shipped masked split lifting it off.
    ///
    /// The field must come through the mask step bit-identical. Ctrl and no
    /// ctrl differ in which map they write, and a ctrl gesture that moved one
    /// voxel of geometry would be a destructive operation wearing a
    /// selection's clothes.
    #[test]
    fn a_ctrl_lasso_selects_a_region_and_it_can_be_split_off() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();

        let before = app.doc.active_volume().duplicated(glam::IVec3::ZERO);
        let bodies_before = app.doc.nodes().len();

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        app.control = true;
        drag_loop(&mut app, &loop_around(SIZE.y * 0.12, 20));
        app.control = false;

        assert!(
            !app.doc.active_volume().mask().is_free(),
            "a ctrl lasso protected nothing: {}",
            app.status
        );
        // Compared by sampling rather than through the core's own helper,
        // which is behind a test-only module this crate cannot reach.
        for probe in [
            Vec3::new(0.0, 0.0, MODEL_RADIUS_MM - 2.0),
            Vec3::new(8.0, 4.0, MODEL_RADIUS_MM - 6.0),
            Vec3::ZERO,
            Vec3::new(-12.0, 0.0, 0.0),
        ] {
            assert_eq!(
                before.sample_world(probe),
                app.doc.active_volume().sample_world(probe),
                "a ctrl lasso moved the field at {probe:?}"
            );
        }
        assert!(app.status.contains("lasso"), "the status does not name the shape: {}", app.status);

        // And the selection lifts off as its own row.
        update(&mut app, Message::BodySplitMasked);
        assert!(
            app.doc.nodes().len() > bodies_before,
            "the masked split produced no new body: {}",
            app.status
        );
    }

    /// **A cut that severs a body says so.**
    ///
    /// Two shells inside one `Volume` look right on screen, export as one
    /// object and arrive at the slicer in two pieces. Nothing before this said
    /// a word about it.
    #[test]
    fn a_cut_that_severs_a_body_reports_the_loose_piece() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();

        // A dumbbell: two balls joined by a thin neck, arranged across the view
        // so a straight drag can part them.
        let eye = app.camera.eye();
        let facing = (app.camera.target - eye).normalize();
        let across = facing.cross(Vec3::Y).normalize_or(Vec3::X);
        {
            let volume = app.doc.active_volume_mut();
            *volume = hani_core::Volume::new(volume.voxel_size());
            volume.seed_sphere(across * -14.0, 10.0);
            union_sphere(volume, across * 14.0, 10.0);
            // The neck, as a chain of overlapping spheres: there is no capsule
            // primitive, and a union of them is the same solid.
            for step in 0..=56 {
                let t = step as f32 / 56.0;
                union_sphere(volume, across * (-14.0 + t * 28.0), 2.5);
            }
            volume.mark_everything_dirty();
        }
        app.rebuild_everything();
        assert_eq!(
            app.doc.split_plan(app.doc.active()).map(|plan| plan.parts.len()),
            Some(1),
            "the fixture is already in pieces before the cut"
        );

        // Cut the neck: a lasso around the middle, deep enough to part it.
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        app.shift = true;
        drag_loop(&mut app, &loop_around(SIZE.y * 0.035, 16));
        app.shift = false;

        assert!(
            app.status.contains("loose piece"),
            "a severed body was not reported: {}",
            app.status
        );
    }

    /// A lasso disarms the tool exactly as a line does. The one-shot property
    /// belongs to the gesture, not to the shape of it.
    #[test]
    fn a_lasso_disarms_the_cut_like_a_line_does() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        drag_loop(&mut app, &loop_around(SIZE.y * 0.15, 20));
        assert_ne!(app.tool, Tool::Cut, "the cut stayed armed after a lasso");
    }

    /// Arm the cut, hold ctrl, and draw a loop: the mask-instead-of-cut gesture.
    ///
    /// The release goes through `update` rather than `on_pointer`, because the
    /// every-message tail is what refreshes the mask card -- and a status line
    /// that names the card's verb row is only honest if that row is up by the
    /// time anyone reads it.
    fn ctrl_lasso(app: &mut Hani, radius: f32) {
        update(app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        app.control = true;
        let points = loop_around(radius, 24);
        let first = *points.first().expect("a loop needs points");
        press(app, first);
        for point in points.iter().skip(1) {
            app.on_pointer(PointerEvent::Moved { position: *point, size: SIZE });
        }
        app.on_pointer(PointerEvent::Moved { position: first, size: SIZE });
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
        app.control = false;
    }

    /// **The status must not send anyone to a panel with no such button.**
    ///
    /// It used to say "split it off from the BODIES panel". The BODIES verb row
    /// is five icons and its Split fires [`Message::BodySplit`], a different
    /// operation, so a user who followed the sentence either found nothing or
    /// found the wrong thing. A beta user hit it within an hour of the
    /// repository going public.
    #[test]
    fn the_ctrl_lasso_status_names_a_surface_that_is_actually_on_screen() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();

        ctrl_lasso(&mut app, SIZE.y * 0.15);

        assert!(!app.doc.active_volume().mask().is_free(), "the ctrl lasso masked nothing");
        assert!(
            !app.status.contains("BODIES"),
            "the status still sends the user to a panel that has no such button: {}",
            app.status
        );
        assert!(
            app.status.contains("mask card"),
            "the status does not name the surface the verb is on: {}",
            app.status
        );
        assert!(
            app.mask_card.as_ref().is_some_and(MaskCard::names_the_active_body),
            "the status named the mask card's verb row and the card will not be showing one: {}",
            app.status
        );
    }

    /// **The advised operation must actually run from the state the gesture
    /// leaves**, which is the half of the claim a test can carry.
    ///
    /// Dispatching [`Message::BodySplitMasked`] with no `ToolChanged` in
    /// between is what pressing the card's `split off` does, so this pins that
    /// the advice is not merely well-worded but possible: the tool is disarmed,
    /// the active body carries a partial mask, and `split_the_mask_off` accepts
    /// it rather than refusing. Before this round the same dispatch would still
    /// have worked -- the defect was that nothing on screen could send it.
    ///
    /// **What this does NOT pin:** that the button exists in the widget tree.
    /// iced offers no introspection, so no test in this suite can see a button.
    /// That half is verified by driving the running application; see
    /// `docs/DRIVING-THE-APP.md`.
    #[test]
    fn a_ctrl_lasso_leaves_the_split_it_advises_ready_to_run() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        let before = app.doc.body_count();

        ctrl_lasso(&mut app, SIZE.y * 0.15);
        assert!(app.status.contains("split off"), "the status did not advise a split");

        update(&mut app, Message::BodySplitMasked);

        assert!(
            app.status.starts_with("split "),
            "the split the status advised refused instead: {}",
            app.status
        );
        assert_eq!(
            app.doc.body_count(),
            before + 1,
            "the advised split did not leave two bodies where there was one"
        );
    }

    /// The advice points at the card only when the card will carry the verb,
    /// and never reads as an offer to lift several bodies off at once.
    #[test]
    fn the_split_advice_points_at_the_card_only_when_the_card_will_carry_the_verb() {
        // The active body was masked, so the card's verb row is up.
        assert!(split_advice(true, 1).starts_with("split off on the mask card"));
        assert!(!split_advice(true, 1).contains("BODIES"));
        assert!(!split_advice(true, 1).contains("one body at a time"));

        // Several bodies masked: one press splits one of them.
        assert!(split_advice(true, 3).contains("one body at a time"));

        // The active body was missed, so BODIES is where a body gets chosen
        // first -- and the singular case must read as singular.
        assert!(split_advice(false, 1).contains("click it in BODIES"));
        assert!(split_advice(false, 3).contains("click one of them in BODIES"));
        for touched in [1, 3] {
            assert!(
                split_advice(false, touched).contains("mask card"),
                "the verb is still on the card even when a body must be chosen first"
            );
        }
    }

    /// **The sentence names the body it masked, not whichever body was active.**
    ///
    /// The loop walks every DRAWN body while `split_the_mask_off` takes
    /// `doc.active()`, and the old line read the second while describing the
    /// first: mask a body that is not active and it confidently named a body
    /// the gesture never went near. Hiding the active one is the cheap way to
    /// make it undrawn -- placing two bodies so a centred loop reaches one and
    /// misses the other is the fiddly way, and buys nothing extra.
    #[test]
    fn a_ctrl_lasso_names_the_body_it_masked_rather_than_the_one_that_was_active() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();

        let sphere = app.doc.active();
        let sphere_name = app.body_name(sphere);

        // A second body, which becomes active, and is then taken off screen so
        // the gesture cannot reach it. Hiding the active body moves the active
        // row to the first visible one -- deliberate, and documented in
        // `toggle_visibility` -- so it has to be selected back afterwards. That
        // reselection is the whole fixture: it is the only way to reach the
        // state where the active body is not the body a gesture can touch.
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let cube = app.doc.active();
        assert_ne!(cube, sphere, "the primitive did not become the active body");
        update(&mut app, Message::BodyVisibilityToggled(cube));
        update(&mut app, Message::BodySelected(cube));
        assert_eq!(app.doc.active(), cube, "the hidden body could not be made active again");

        ctrl_lasso(&mut app, SIZE.y * 0.15);

        assert!(
            app.status.contains(&sphere_name),
            "the status did not name the body the lasso actually masked: {}",
            app.status
        );
        assert!(
            app.status.contains("click it in BODIES"),
            "the status offered a card verb for a body that was not masked: {}",
            app.status
        );
    }

    /// The status names the shape, because "cut 812 bricks" over a lasso and
    /// over a plane describe very different things happening to the model.
    #[test]
    fn the_status_says_which_shape_cut() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        drag_loop(&mut app, &loop_around(SIZE.y * 0.15, 24));
        assert!(app.status.contains("lasso"), "a lasso reported: {}", app.status);

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        press(&mut app, Vector::new(SIZE.x * 0.1, SIZE.y * 0.3));
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.9, SIZE.y * 0.3),
            size: SIZE,
        });
        release(&mut app);
        assert!(!app.status.contains("lasso"), "a straight drag reported: {}", app.status);
    }

    /// A loop thinner than the lattice is refused with its own sentence rather
    /// than reporting a successful cut over a model that did not change.
    #[test]
    fn a_loop_thinner_than_the_voxel_grid_is_refused() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        let before = app.doc.active_volume().brick_coords().count();

        // Long, and about a pixel wide: out along the middle of the viewport
        // and back a hair below.
        let middle = SIZE.y * 0.5;
        let mut path = vec![Vector::new(SIZE.x * 0.2, middle)];
        for step in 1..=20 {
            path.push(Vector::new(SIZE.x * (0.2 + 0.03 * step as f32), middle));
        }
        for step in (0..=20).rev() {
            path.push(Vector::new(SIZE.x * (0.2 + 0.03 * step as f32), middle + 0.4));
        }
        drag_loop(&mut app, &path);

        assert_eq!(
            app.doc.active_volume().brick_coords().count(),
            before,
            "a sliver loop changed the model"
        );
        assert!(app.status.contains("cancelled"), "reported: {}", app.status);
    }

    /// Escape mid-drag throws the shape away and leaves the tool armed, which
    /// is the treatment a live gizmo drag already gets.
    #[test]
    fn escape_mid_path_leaves_the_cut_armed() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        press(&mut app, Vector::new(SIZE.x * 0.2, SIZE.y * 0.5));
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.5, SIZE.y * 0.4),
            size: SIZE,
        });
        assert!(!app.cut_path.is_empty(), "the path did not accumulate");

        update(&mut app, Message::MenuClosed);

        assert_eq!(app.tool, Tool::Cut, "escape mid-path disarmed the tool");
        assert!(app.cut_path.is_empty(), "escape left the path behind");
        assert!(!app.unsaved, "escape mid-path still cut");
    }

    /// The tool card has to say what the gesture in progress will do. "PLANE
    /// CUT" over a lasso is a lie about which material is about to go.
    #[test]
    fn the_live_label_follows_the_shape_being_drawn() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        assert_eq!(app.live_tool_label(), "PLANE CUT", "an unstarted cut is not a plane cut");

        let points = loop_around(SIZE.y * 0.15, 20);
        press(&mut app, points[0]);
        for point in points.iter().skip(1) {
            app.on_pointer(PointerEvent::Moved { position: *point, size: SIZE });
        }
        app.on_pointer(PointerEvent::Moved { position: points[0], size: SIZE });
        assert_eq!(app.live_tool_label(), "LASSO CUT", "a live loop still says plane cut");
    }

    #[test]
    fn a_cut_is_undoable_through_the_application() {
        let mut app = app();
        let probe = Vec3::new(0.0, 12.0, 0.0);
        let before = app.doc.active_volume().sample_world(probe);

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        press(&mut app, Vector::new(SIZE.x * 0.1, SIZE.y / 2.0));
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.9, SIZE.y / 2.0),
            size: SIZE,
        });
        release(&mut app);
        assert_ne!(
            app.doc.active_volume().sample_world(probe),
            before,
            "the cut did nothing to undo"
        );

        app.undo();
        assert_eq!(
            app.doc.active_volume().sample_world(probe),
            before,
            "undo did not restore the cut"
        );
    }

    /// A cut a mask blocked entirely must never report itself as a cut that
    /// missed.
    ///
    /// This is the one failure masking adds to a tool that had none: the cut
    /// does nothing, and the reason is protection painted minutes ago, possibly
    /// on a body that is not even the one being looked at. "The cut missed the
    /// model" sends the user off to redraw a line that was never the problem,
    /// so the message names the mask AND the body it is on.
    #[test]
    fn a_cut_a_mask_blocked_names_the_mask_and_the_body_it_is_on() {
        let mut app = app();
        let body = app.doc.active();
        let mut meta = app.doc.meta(body).expect("the active body");
        meta.name = "Left Ear".to_string();
        app.doc.set_meta(&meta);
        // Mask All: an empty map read inverted, so every voxel of the body is
        // protected and it costs nothing.
        app.doc.active_volume_mut().mask_mut().set_inverted(true);

        let before = app.doc.active_volume().brick_coords().count();
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        press(&mut app, Vector::new(SIZE.x * 0.1, SIZE.y / 2.0));
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.9, SIZE.y / 2.0),
            size: SIZE,
        });
        release(&mut app);

        assert_eq!(
            app.doc.active_volume().brick_coords().count(),
            before,
            "the mask did not stop the cut"
        );
        assert!(!app.unsaved, "a cut that changed nothing marked the document unsaved");
        assert!(
            !app.status.contains("missed"),
            "a cut a mask blocked reported that it missed: {}",
            app.status
        );
        assert!(app.status.contains("mask"), "the message does not name the mask: {}", app.status);
        assert!(
            app.status.contains("Left Ear"),
            "the message does not name the body the mask is on: {}",
            app.status
        );
    }

    /// Escape is the way out of every other mode, and a destructive one must
    /// not be the exception.
    #[test]
    fn escape_disarms_the_cut() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        assert_eq!(app.tool, Tool::Cut);
        update(&mut app, Message::MenuClosed);
        assert_ne!(app.tool, Tool::Cut, "escape left a destructive mode armed");
    }

    // --- importing a mesh ----------------------------------------------------

    /// Drives the import through the application rather than the voxeliser,
    /// because the interesting failures are all in the wiring: a stale mesh
    /// pool, history outliving its model, and above all a document that thinks
    /// it belongs to the STL it came from.
    #[test]
    fn importing_a_mesh_replaces_the_model_without_claiming_its_file() {
        let directory = scratch("import");
        let path = directory.join("cube.stl");

        // Write a real STL through this project's own writer, so the test
        // exercises the reader rather than an assumption about the format.
        let mut source = hani_core::Volume::new(0.5);
        source.seed_sphere(Vec3::ZERO, 14.0);
        source.mark_everything_dirty();
        let (mesh, report) = source.export_mesh();
        assert!(report.is_printable(), "the fixture is not printable");
        let mut bytes = Vec::new();
        hani_core::export::stl::write(&mesh, &mut bytes).unwrap();
        std::fs::write(&path, &bytes).unwrap();

        let mut app = app();
        app.project_path = Some(directory.join("previous.hani"));
        press(&mut app, centre_of_viewport());
        release(&mut app);

        let imported = hani_core::import::read_path(&path).expect("the STL should read");
        let options = hani_core::voxelise::VoxeliseOptions::at(app.doc.voxel_size());
        let (volume, voxel_report) =
            hani_core::voxelise::voxelise(&imported, &options).expect("it should voxelise");
        app.adopt_import(crate::message::Imported {
            volume,
            report: voxel_report,
            source: path.clone(),
            elapsed_ms: 0.0,
            resting_up: hani_core::resting_up(&imported.positions),
        });

        assert!(
            app.project_path.is_none(),
            "the import adopted the mesh's path, so the next plain Save would write a .hani \
             container straight over the user's .stl"
        );
        assert!(app.unsaved, "an imported model is unsaved work and quitting would lose it");
        assert!(!app.history.can_undo(), "history outlived the model it belonged to");
        assert!(app.perf.dirty_bricks > 0, "nothing was meshed, so the import is invisible");
        assert!(app.status.starts_with("imported"), "reported: {}", app.status);

        let (_, after) = app.doc.active_volume().export_mesh();
        assert!(after.is_printable(), "the imported model is not printable: {}", after.summary());

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A file that cannot be read must leave the sculpt on screen alone, and
    /// must say so in a way the header renders as an error.
    #[test]
    fn a_mesh_that_cannot_be_read_reports_and_changes_nothing() {
        let directory = scratch("import-bad");
        let path = directory.join("broken.stl");
        std::fs::write(&path, b"this is not an STL of any flavour").unwrap();

        let mut app = app();
        press(&mut app, centre_of_viewport());
        release(&mut app);
        let before = app.doc.active_volume().sample_world(Vec3::ZERO);

        let payload = crate::message::ImportPayload::new(
            hani_core::import::read_path(&path).map(|_| unreachable!("it should not parse")),
        );
        update(&mut app, Message::ImportLoaded(payload));

        assert!(
            app.status.contains("could not"),
            "the failure does not contain the substring the header colours as an error: {}",
            app.status
        );
        assert_eq!(
            app.doc.active_volume().sample_world(Vec3::ZERO),
            before,
            "a failed import changed the model"
        );
        assert!(app.history.can_undo(), "a failed import cleared history");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// Import discards the document, so it goes through the same gate as Open.
    #[test]
    fn importing_with_unsaved_work_raises_the_prompt() {
        let mut app = app_with_unsaved_work();
        update(&mut app, Message::ImportRequested);
        assert_eq!(app.confirm, Some(PendingAction::Import));
    }

    /// **A saved multi-body document is still work, and import throws all of it
    /// away.**
    ///
    /// The gate consulted `unsaved` alone, so with a saved five-body project
    /// open, File > Import replaced every body with the mesh and asked nothing:
    /// no dialog, no prompt, no status line. The file survives on disk, which
    /// is why this is a prompt rather than a refusal.
    #[test]
    fn importing_over_a_saved_document_of_several_bodies_asks_first() {
        let mut app = app();
        assert!(!app.unsaved, "the fixture must be clean or this measures the old gate");

        // One body and saved: nothing to lose that Open would not also cost, so
        // no prompt. This half is what says the guard did not simply become
        // "always ask".
        update(&mut app, Message::ImportRequested);
        assert_eq!(app.confirm, None, "a single-body import should not be interrupted");

        let mut second = Volume::new(app.doc.voxel_size());
        second.seed_sphere(Vec3::new(60.0, 0.0, 0.0), 10.0);
        app.doc.add_body("Body 2", second);
        app.publish_visibility();
        assert!(!app.unsaved, "adding the fixture body must not be what raises the prompt");

        update(&mut app, Message::ImportRequested);
        assert_eq!(
            app.confirm,
            Some(PendingAction::Import),
            "importing discarded a second body without asking"
        );
    }

    /// Two bodies, one in front of the other, both drawn.
    ///
    /// The second ball is placed on the line from the origin to the eye, so a
    /// press at the centre of the viewport meets it before the starting sphere.
    /// `publish_visibility` is called by hand because the fixture builds the
    /// document directly rather than through a message, and the pick reads the
    /// mask the application publishes.
    fn two_bodies_in_a_line(app: &mut Hani) -> (NodeId, NodeId) {
        let front = app.camera.eye().normalize() * 55.0;
        let mut second = Volume::new(app.doc.voxel_size());
        second.seed_sphere(front, 8.0);
        second.mark_everything_dirty();
        let other = app.doc.add_body("Body 2", second);
        app.publish_visibility();
        app.remesh_dirty();
        (app.doc.active(), other)
    }

    /// **The press ordering, which is the one that loses work if it is wrong.**
    ///
    /// Recording used to open in the `Pressed` arm before anything had been
    /// raycast. With two bodies that opens it on the wrong one, and
    /// `record_for_undo` does nothing at all when the volume it is called on
    /// has no recorder — so the carve lands, no entry is pushed, `unsaved` stays
    /// false, and quitting does not even raise the discard prompt.
    ///
    /// The rule that falls out is the one Photoshop's layer list has: a press
    /// on something that is not selected selects it, and the press after that
    /// works on it.
    #[test]
    fn a_press_over_another_body_selects_it_and_carves_nothing_until_the_next_press() {
        let mut app = app();
        let (first, other) = two_bodies_in_a_line(&mut app);

        // The surfaces the camera meets first: the starting ball at its own
        // radius, and the second ball at its centre plus its radius, because
        // the ray comes inwards from the eye.
        let front_of_first = app.camera.eye().normalize() * MODEL_RADIUS_MM;
        let front_of_other = app.camera.eye().normalize() * 63.0;
        let first_before = app.doc.volume(first).expect("a live body").sample_world(front_of_first);
        let other_before = app.doc.volume(other).expect("a live body").sample_world(front_of_other);

        press(&mut app, centre_of_viewport());
        release(&mut app);

        assert_eq!(app.doc.active(), other, "the press did not choose the body it landed on");
        assert!(!app.history.can_undo(), "a press that only selected recorded a stroke");
        assert!(!app.unsaved, "a press that only selected dirtied the document");
        assert_eq!(
            app.doc.volume(first).expect("a live body").sample_world(front_of_first),
            first_before,
            "the press carved the body it was leaving"
        );
        assert_eq!(
            app.doc.volume(other).expect("a live body").sample_world(front_of_other),
            other_before,
            "the press that selected also carved"
        );

        // And now it sculpts, on the body it chose.
        press(&mut app, centre_of_viewport());
        release(&mut app);

        assert_eq!(app.doc.active(), other, "the second press moved the selection again");
        assert!(app.history.can_undo(), "the second press recorded no undo entry");
        assert!(app.unsaved, "the second press left the document looking saved");
        assert_ne!(
            app.doc.volume(other).expect("a live body").sample_world(front_of_other),
            other_before,
            "the second press did not carve the selected body"
        );
        assert_eq!(
            app.doc.volume(first).expect("a live body").sample_world(front_of_first),
            first_before,
            "the stroke landed on the body that was NOT selected"
        );
    }

    /// Hiding is a draw-time skip, so the depth buffer where a hidden body sits
    /// is empty. A press there must go through it: carving something invisible
    /// would set `unsaved`, push an entry, pay a remesh and an upload, and
    /// change not one pixel.
    #[test]
    fn a_press_over_a_hidden_body_carves_nothing_and_reaches_what_is_behind_it() {
        let mut app = app();
        let (first, other) = two_bodies_in_a_line(&mut app);

        let mut meta = app.doc.meta(other).expect("the second body");
        meta.visible = false;
        app.doc.set_meta(&meta);
        app.publish_visibility();

        let front_of_first = app.camera.eye().normalize() * MODEL_RADIUS_MM;
        let front_of_other = app.camera.eye().normalize() * 63.0;
        let other_before = app.doc.volume(other).expect("a live body").sample_world(front_of_other);
        let first_before = app.doc.volume(first).expect("a live body").sample_world(front_of_first);

        press(&mut app, centre_of_viewport());
        release(&mut app);

        assert_eq!(app.doc.active(), first, "a hidden body was selected by a press over it");
        assert_eq!(
            app.doc.volume(other).expect("a live body").sample_world(front_of_other),
            other_before,
            "the press carved a body that is not on screen"
        );
        assert_ne!(
            app.doc.volume(first).expect("a live body").sample_world(front_of_first),
            first_before,
            "the press stopped at a body nothing is drawing"
        );
    }

    /// The same rule, for the case the pick cannot answer on its own.
    ///
    /// [`Document::pick`] masks on visibility, so it refuses a hidden body --
    /// but a stroke takes its surface from `pick_body`, which asks one named
    /// body and never consults the eye. So a press while the ACTIVE body is
    /// hidden picked nothing, fell through to a sculpt, and carved the
    /// invisible body: an undo entry pushed, `unsaved` set, a remesh and an
    /// upload paid for, and not one pixel changed. Reachable today only by
    /// opening a file whose active row is hidden; a one-click bug the moment
    /// the panel ships an eye.
    #[test]
    fn a_press_while_the_active_body_is_hidden_carves_nothing_and_says_which_body() {
        let mut app = app();
        let active = app.doc.active();
        let mut meta = app.doc.meta(active).expect("the starting body");
        meta.visible = false;
        app.doc.set_meta(&meta);
        app.publish_visibility();

        let front = app.camera.eye().normalize() * MODEL_RADIUS_MM;
        let before = app.doc.active_volume().sample_world(front);

        press(&mut app, centre_of_viewport());
        app.on_pointer(PointerEvent::Moved {
            position: centre_of_viewport() + Vector::new(20.0, 0.0),
            size: SIZE,
        });
        release(&mut app);

        assert_eq!(
            app.doc.active_volume().sample_world(front),
            before,
            "the press carved the body that is not on screen"
        );
        assert!(!app.history.can_undo(), "a press that carved nothing recorded a stroke");
        assert!(!app.unsaved, "a press that carved nothing dirtied the document");
        assert!(
            app.status.contains("hidden") && app.status.contains(Document::FIRST_BODY_NAME),
            "nothing said why the press did nothing: {:?}",
            app.status
        );

        // Shown again, the very same press works: the refusal is about the
        // eye and nothing else.
        meta.visible = true;
        app.doc.set_meta(&meta);
        app.publish_visibility();
        press(&mut app, centre_of_viewport());
        release(&mut app);
        assert_ne!(
            app.doc.active_volume().sample_world(front),
            before,
            "the press stopped working on a body that is drawn"
        );
    }

    /// **A live stroke owns the ring.**
    ///
    /// The `Moved` arm carved the active body and then picked across every
    /// drawn body anyway, so a stroke dragged over a second body moved
    /// `hover_body` onto that second body -- and the ring is built from the
    /// hovered body's field and coloured `CursorMood::Selecting` whenever the
    /// hovered body is not the active one. The cursor said "a press here would
    /// select", on the wrong surface, during a press that was carving
    /// something else.
    #[test]
    fn the_ring_stays_on_the_body_a_live_stroke_is_carving() {
        let mut app = app();
        let (first, other) = two_bodies_in_a_line(&mut app);

        // The control. With no button down the centre of the viewport hovers
        // the SECOND body, which is the whole reason a stroke dragged there
        // could move the ring; without this the test would also pass on a
        // build where the hover never reaches that body at all.
        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        assert_eq!(app.hover_body, Some(other), "the fixture never hovers the second body");

        // Off to the side, where only the first body is, so the press starts a
        // stroke instead of choosing the other body.
        let beside = Vector::new(610.0, 300.0);
        press(&mut app, beside);
        assert_eq!(app.doc.active(), first, "the press beside the second ball chose a body");
        assert!(
            matches!(app.drag.map(|drag| drag.kind), Some(DragKind::Sculpt(_))),
            "the press beside the second ball did not start a stroke: {:?}",
            app.drag.map(|drag| drag.kind)
        );

        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        assert_eq!(
            app.hover_body,
            Some(first),
            "mid-stroke the ring hopped onto a body the stroke is not carving"
        );
        assert!(app.hover.is_some(), "the ring vanished from the body being carved");

        // And it comes back the moment the stroke ends.
        release(&mut app);
        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        assert_eq!(app.hover_body, Some(other), "the hover never returned to the pick");
    }

    /// **The mitigation for the mirror centre being the lattice origin.**
    ///
    /// With the centre pinned there, mirroring a body that sits wholly to one
    /// side of a plane writes its twin out in empty space: a free-floating
    /// shell that exports and that no slicer can print. The axis is refused
    /// rather than enabled-and-ignored, so there is one answer to "is X on".
    #[test]
    fn a_mirror_the_active_body_does_not_straddle_is_refused_with_a_reason() {
        let mut app = app();

        // The starting ball is centred on the origin, so every plane crosses it
        // and every axis is allowed. This half is what stops the refusal from
        // being "always refuse".
        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        assert!(
            app.symmetry_settings().axis(MirrorAxis::X),
            "a body on the origin was refused its mirror"
        );
        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        assert!(!app.symmetry_settings().axis(MirrorAxis::X));

        let mut second = Volume::new(app.doc.voxel_size());
        second.seed_sphere(Vec3::new(80.0, 0.0, 0.0), 10.0);
        second.mark_everything_dirty();
        let other = app.doc.add_body("Body 2", second);
        app.publish_visibility();
        app.remesh_dirty();
        app.select_body(other);

        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        assert!(
            !app.symmetry_settings().axis(MirrorAxis::X),
            "a mirror was enabled that misses the body"
        );
        assert!(
            app.status.contains('X') && app.status.contains("Body 2"),
            "the refusal said nothing useful: {:?}",
            app.status
        );

        // The other two planes DO cross it, and must not be refused along with
        // the one that does not.
        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::Y));
        assert!(
            app.symmetry_settings().axis(MirrorAxis::Y),
            "a plane the body straddles was refused"
        );
    }

    /// **A refusal wired into one event site is not a refusal.**
    ///
    /// The symmetry strip is not the only way to turn X mirroring on: a
    /// SpaceMouse button bound to `ToggleSymmetry` did it with a bare
    /// `Symmetry::toggled`, so a puck user on a body at x = +80 got every
    /// stroke's twin written into empty space, with the strip highlight and
    /// the drawn plane both agreeing that nothing was wrong. This drives the
    /// application's real button path rather than calling the gate, because
    /// the gate was never what was broken.
    #[test]
    fn the_spacemouse_symmetry_button_is_refused_the_mirror_the_strip_is_refused() {
        let mut app = app();
        let first = app.doc.active();
        app.spacemouse.config.buttons[0] = ButtonAction::ToggleSymmetry;

        let mut second = Volume::new(app.doc.voxel_size());
        second.seed_sphere(Vec3::new(80.0, 0.0, 0.0), 10.0);
        second.mark_everything_dirty();
        let other = app.doc.add_body("Body 2", second);
        app.publish_visibility();
        app.remesh_dirty();
        app.select_body(other);

        app.spacemouse.simulate_press(0);
        app.drive_from_spacemouse(16.0);

        assert!(
            !app.symmetry_settings().axis(MirrorAxis::X),
            "the puck turned on a mirror plane the body sits entirely to one side of"
        );
        assert!(
            app.status.contains('X') && app.status.contains("Body 2"),
            "the puck's refusal said nothing: {:?}",
            app.status
        );

        // And it still works where the mirror is legitimate, so the fix is a
        // gate and not a disconnected wire.
        app.select_body(first);
        app.spacemouse.simulate_press(0);
        app.drive_from_spacemouse(16.0);
        assert!(
            app.symmetry_settings().axis(MirrorAxis::X),
            "the puck stopped turning on a mirror the body does straddle"
        );
    }

    /// A view out of a file is a third way for a mirror to come on, and a
    /// restored one carves twins into empty space exactly as an enabled one
    /// does -- the file may have been written by a build without the refusal,
    /// or with a different body active.
    #[test]
    fn a_mirror_restored_from_a_view_the_body_does_not_straddle_is_turned_off() {
        let mut app = app();
        let mut off_centre = Volume::new(app.doc.voxel_size());
        off_centre.seed_sphere(Vec3::new(80.0, 0.0, 0.0), 10.0);
        off_centre.mark_everything_dirty();
        app.doc = Document::from_volume(off_centre);
        app.publish_visibility();
        app.remesh_dirty();

        let mut view = app.current_view();
        view.mirror = [true, true, true];
        app.apply_view(&view);

        assert!(
            !app.symmetry_settings().axis(MirrorAxis::X),
            "a file turned on a mirror the body misses"
        );
        assert!(
            app.symmetry_settings().axis(MirrorAxis::Y)
                && app.symmetry_settings().axis(MirrorAxis::Z),
            "the two planes the body does straddle were dropped as well"
        );
    }

    /// Each body has its own planes (`app/symmetry_frame.rs`): choosing a
    /// body away from the first shows that body's, which are off, and the
    /// first keeps its own. Until each body had its own settings the plane
    /// was the session's, and choosing a body it missed turned it off and
    /// said so.
    #[test]
    fn selecting_another_body_shows_its_own_planes_and_the_first_keeps_its() {
        let mut app = app();
        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        assert!(app.symmetry_settings().axis(MirrorAxis::X));

        let mut second = Volume::new(app.doc.voxel_size());
        second.seed_sphere(Vec3::new(80.0, 0.0, 0.0), 10.0);
        second.mark_everything_dirty();
        let other = app.doc.add_body("Body 2", second);
        app.publish_visibility();
        app.remesh_dirty();

        let first = app.doc.active();
        app.select_body(other);
        assert!(
            !app.symmetry_settings().axis(MirrorAxis::X),
            "the first body's mirror showed on the second"
        );
        assert!(app.doc.symmetry(first).axis(MirrorAxis::X), "the first body lost its mirror");
        app.select_body(first);
        assert!(
            app.symmetry_settings().axis(MirrorAxis::X),
            "the first body's mirror is not shown"
        );
    }

    /// **A quarter turn turns the whole document.**
    ///
    /// Bodies share one lattice and have no transform, so their arrangement is
    /// the only positional state there is. `orient` turned `active_volume()`
    /// alone, which with two bodies scatters that arrangement — and makes its
    /// own status line, "turn it back the same way to undo", untrue for every
    /// body that stayed put. The bit-exactness of four turns is measured in
    /// `hani-core`; this is the wiring.
    #[test]
    fn turning_the_model_turns_every_body_and_not_just_the_active_one() {
        let mut app = app();
        // Off the axis the turn is about: a ball sitting ON that axis comes
        // back to the same box whether the turn reached it or not, and this
        // test would then pass with `orient` still turning one body.
        let mut second = Volume::new(app.doc.voxel_size());
        second.seed_sphere(Vec3::new(0.0, 50.0, 0.0), 8.0);
        second.mark_everything_dirty();
        let other = app.doc.add_body("Body 2", second);
        app.publish_visibility();
        app.remesh_dirty();

        let before =
            app.doc.volume(other).expect("a live body").world_bounds().expect("a seeded ball");
        app.orient(hani_core::AxisRotation::taking(
            hani_core::Facing::Front,
            hani_core::Facing::Up,
        ));
        let after =
            app.doc.volume(other).expect("a live body").world_bounds().expect("still a ball");

        assert_ne!(before, after, "the inactive body did not turn with the model");
        assert!(
            ((after.1 - after.0).length() - (before.1 - before.0).length()).abs() < 1.0e-3,
            "the inactive body changed size rather than orientation: {before:?} to {after:?}"
        );
    }

    #[test]
    fn the_autosave_tick_waits_for_the_interval_and_for_the_stroke_to_end() {
        let directory = scratch("autosave-tick");
        let mut app = app();
        app.autosave_file = Some(directory.join("autosave.hani"));

        // Clean document: nothing to protect, so nothing is written however
        // long it has been.
        app.last_autosave = Instant::now() - Duration::from_secs(600);
        app.maybe_autosave();
        assert!(!app.has_autosave(), "a clean document was autosaved");

        // Dirty, but the interval has not elapsed.
        press(&mut app, centre_of_viewport());
        release(&mut app);
        app.last_autosave = Instant::now();
        app.maybe_autosave();
        assert!(!app.has_autosave(), "it autosaved before the interval was up");

        // Dirty and overdue, but mid-drag: a write here reads as a stutter in
        // the brush.
        app.last_autosave = Instant::now() - Duration::from_secs(600);
        press(&mut app, centre_of_viewport());
        app.maybe_autosave();
        assert!(!app.has_autosave(), "it autosaved in the middle of a stroke");

        // Released, dirty and overdue -- but the pointer only just stopped, so
        // the write still waits for the pause.
        release(&mut app);
        app.maybe_autosave();
        assert!(
            !app.has_autosave(),
            "it autosaved the instant the pointer stopped, which is where a 400 ms write \
             lands as a visible hitch"
        );

        // Once the pointer has been still long enough, it writes: a piece a
        // tick, on another thread. See `app/autosave.rs`.
        app.last_activity = Instant::now() - Duration::from_secs(60);
        app.maybe_autosave();
        while app.autosave.in_flight() {
            app.maybe_autosave();
        }
        app.autosave.settle();
        assert!(app.has_autosave(), "an overdue autosave never happened after a pause");

        std::fs::remove_dir_all(&directory).ok();
    }
    #[test]
    fn a_bad_file_reports_and_changes_nothing() {
        let directory = std::env::temp_dir().join(format!("hani-bad-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("not-a-sculpt.hani");
        std::fs::write(&path, b"this is not a sculpt").unwrap();

        let mut app = app();
        let before = app.doc.active_volume().brick_coords().count();
        app.open_project(&path);

        assert!(app.status.contains("not a HaniSculpt file"), "reported: {}", app.status);
        assert_eq!(
            app.doc.active_volume().brick_coords().count(),
            before,
            "a failed open lost the model"
        );
        assert!(app.project_path.is_none(), "a failed open claimed the file");

        app.open_project(&directory.join("does-not-exist.hani"));
        assert!(app.status.contains("could not open"), "reported: {}", app.status);
        assert_eq!(app.doc.active_volume().brick_coords().count(), before);

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_dynamic_radius_keeps_its_proportion_to_the_model() {
        let proportion = |state: &Hani| state.brush.radius / state.model_radius;

        // Off by default: the radius means a physical size, so a resample must
        // leave it alone.
        let mut fixed = app();
        let before = fixed.brush.radius;
        update(&mut fixed, Message::Resample(VOXEL_SIZE_MM / 2.0));
        assert_eq!(fixed.brush.radius, before, "a fixed radius must survive a resample");

        // On, and the model doubles: the brush should follow it.
        let mut dynamic = app();
        update(&mut dynamic, Message::DynamicRadiusToggled(true));
        let before = proportion(&dynamic);
        dynamic.doc.active_volume_mut().seed_sphere(Vec3::ZERO, MODEL_RADIUS_MM * 2.0);
        dynamic.doc.active_volume_mut().mark_everything_dirty();
        dynamic.remesh_dirty();

        assert!(
            dynamic.brush.radius > fixed.brush.radius,
            "a dynamic radius should have grown with the model"
        );
        assert!(
            (proportion(&dynamic) - before).abs() < 0.15 * before,
            "the brush lost its proportion: {before} became {}",
            proportion(&dynamic)
        );
    }

    /// The same ball, moved bodily away from the origin, must report the same
    /// size.
    ///
    /// This is what `content_radius` replaced `bounding_radius` for. The old
    /// measure was taken from the WORLD ORIGIN, so a model 128 mm out reported
    /// roughly 2.6 times its own radius and the Dynamic brush went with it --
    /// and nothing in the interface would have said why the brush had grown.
    ///
    /// 128 mm rather than a round 100 because at the 0.25 mm voxel that is
    /// exactly sixteen bricks, so the sphere's brick footprint is an exact
    /// translation of the one at the origin and the two radii are identical to
    /// the last bit. That is the shared-lattice property this whole design
    /// rests on, exercised from the application's side.
    #[test]
    fn a_body_far_from_the_origin_does_not_resize_the_dynamic_brush() {
        let mut app = app();
        update(&mut app, Message::DynamicRadiusToggled(true));
        let radius = app.brush.radius;
        let at_origin = app.model_radius;

        let mut moved = hani_core::Volume::new(app.doc.voxel_size());
        moved.seed_sphere(Vec3::new(128.0, 0.0, 0.0), MODEL_RADIUS_MM);
        moved.mark_everything_dirty();
        app.doc.replace_active_volume(moved);
        app.remesh_dirty();

        assert_eq!(
            app.model_radius, at_origin,
            "the same ball measured {} at the origin and {} at 128 mm",
            at_origin, app.model_radius
        );
        assert_eq!(app.brush.radius, radius, "moving the model resized the brush");
    }

    /// Selecting a different body changes no geometry, so it must not resize
    /// the Dynamic brush -- not on the selection, and not on the next remesh
    /// either, which is where a naive implementation would catch up on it a
    /// frame later.
    ///
    /// The fixture is the one from the decision: a small body beside a large
    /// one. Under a plain `previous == self.model_radius` comparison this took
    /// a 3 mm brush to the 0.25 mm floor and then to 10 mm on the way back --
    /// and the press that resizes it is the same press that selects, so the
    /// next press would carve at a radius nobody chose.
    #[test]
    fn changing_the_active_body_does_not_resize_the_dynamic_brush() {
        let mut app = app();
        update(&mut app, Message::DynamicRadiusToggled(true));
        let radius = app.brush.radius;

        let mut rivet = hani_core::Volume::new(app.doc.voxel_size());
        rivet.seed_sphere(Vec3::new(128.0, 0.0, 0.0), 2.5);
        rivet.mark_everything_dirty();
        let second = app.doc.add_body("Body 2", rivet);
        app.remesh_dirty();
        assert_eq!(app.brush.radius, radius, "ADDING a body resized the brush");

        app.doc.set_active(second);
        // Something then dirties a brick -- a stroke, an undo, a resample,
        // anything -- and the remesh that follows measures the rivet where the
        // last measurement was of the ball. That comparison is the whole
        // hazard, so the test has to reach it rather than land on
        // `remesh_dirty`'s empty-set early return.
        app.doc.active_volume_mut().mark_everything_dirty();
        app.remesh_dirty();
        assert_eq!(app.brush.radius, radius, "selecting a smaller body resized the brush");
        assert!(
            app.model_radius < MODEL_RADIUS_MM,
            "the framing radius did not follow the selection: {}",
            app.model_radius
        );

        let first = app.doc.nodes()[0].id;
        app.doc.set_active(first);
        app.doc.active_volume_mut().mark_everything_dirty();
        app.remesh_dirty();
        assert_eq!(app.brush.radius, radius, "selecting back resized the brush");
    }

    /// **The plane cut crosses every VISIBLE body.**
    ///
    /// A cut is a line the user draws across what they can see, so it acts on
    /// what is drawn. `finish_cut` was single-volume before there was a
    /// document, and the rename that gave it one made it silently mean "the
    /// active body" -- which is exactly the kind of change that ships with
    /// nothing failing. This was written as the executable statement of what it
    /// owed and left `#[ignore]`d, alongside a `debug_assert!` at the top of
    /// `finish_cut` saying the same thing from the other side. Both were
    /// discharged by `Document::clip`, which brackets each visible body and
    /// records ONE `Entry` of N `Change::Bricks`.
    #[test]
    fn a_cut_crosses_every_visible_body() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();

        // A second ball overlapping the first, so one screen-space line crosses
        // both of them.
        let mut second = hani_core::Volume::new(app.doc.voxel_size());
        second.seed_sphere(Vec3::new(10.0, 0.0, 0.0), MODEL_RADIUS_MM * 0.5);
        second.mark_everything_dirty();
        let other = app.doc.add_body("Body 2", second);
        app.remesh_dirty();

        let above = Vec3::new(10.0, 12.0, 0.0);
        let before = app.doc.volume(other).expect("a live body").sample_world(above);

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        press(&mut app, Vector::new(SIZE.x * 0.1, SIZE.y / 2.0));
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.9, SIZE.y / 2.0),
            size: SIZE,
        });
        release(&mut app);

        assert_ne!(
            app.doc.volume(other).expect("a live body").sample_world(above),
            before,
            "the cut passed straight through the inactive body and left it whole"
        );
    }

    /// **Solo narrows the cut**, and that is decided rather than incidental.
    ///
    /// The instinct is that a view mode must never narrow an operation. It does
    /// not survive the case: eight bodies with three eye-hidden and four
    /// solo-hidden means a line drawn across one body cuts five, and **nothing
    /// on screen distinguishes an eye-hidden body from a solo-hidden one** --
    /// in both cases it is simply not there. The rule that survives is narrower
    /// and can be stated in one sentence: direct manipulation acts on what is
    /// drawn; the file and the export act on `saved_visibility`.
    #[test]
    fn solo_narrows_the_cut_to_the_body_it_is_showing() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();

        let mut second = hani_core::Volume::new(app.doc.voxel_size());
        second.seed_sphere(Vec3::new(10.0, 0.0, 0.0), MODEL_RADIUS_MM * 0.5);
        second.mark_everything_dirty();
        let other = app.doc.add_body("Body 2", second);
        app.remesh_dirty();

        let first = app.doc.nodes()[0].id;
        update(&mut app, Message::SoloEntered(first));

        let above = Vec3::new(10.0, 12.0, 0.0);
        let spared = app.doc.volume(other).expect("a live body").sample_world(above);
        let over_the_soloed = Vec3::new(0.0, 12.0, 0.0);
        let cut_away = app.doc.volume(first).expect("a live body").sample_world(over_the_soloed);

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        press(&mut app, Vector::new(SIZE.x * 0.1, SIZE.y / 2.0));
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.9, SIZE.y / 2.0),
            size: SIZE,
        });
        release(&mut app);

        assert_eq!(
            app.doc.volume(other).expect("a live body").sample_world(above),
            spared,
            "the cut went through a body solo was hiding"
        );
        // ...and the body solo IS showing really was cut, so this is not a test
        // of a cut that missed everything and passed for the wrong reason.
        assert_ne!(
            app.doc.volume(first).expect("a live body").sample_world(over_the_soloed),
            cut_away,
            "the cut missed the soloed body too: {}",
            app.status
        );
        assert!(
            app.status.contains("bricks") && !app.status.contains("bodies"),
            "the cut did not report exactly one body: {}",
            app.status
        );
    }

    /// Clicking the cube must move the camera and must NOT sculpt. Getting that
    /// wrong takes a divot out of the model every time someone reaches for a
    /// standard view.
    #[test]
    fn clicking_the_navigation_cube_flies_the_camera_without_sculpting() {
        let mut app = app();
        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        let probe = app
            .surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0))
            .expect("the centre should hit the sphere");
        let before_field = app.doc.active_volume().sample_world(probe);
        let before_yaw = app.camera.yaw;

        let (origin, size) = crate::navcube::corner_rect(Vec2::new(SIZE.x, SIZE.y));
        let middle = origin + size * 0.5;
        press(&mut app, Vector::new(middle.x, middle.y));
        release(&mut app);

        assert!(app.flight.is_some(), "clicking the cube did not start a move");
        assert_eq!(
            app.doc.active_volume().sample_world(probe),
            before_field,
            "clicking the cube carved the model"
        );
        assert!(!app.history.can_undo(), "clicking the cube recorded an undo entry");
        assert!(app.drag.is_none(), "clicking the cube started a drag");

        finish_flight(&mut app);
        assert!(app.flight.is_none(), "the move never finished");
        assert_ne!(app.camera.yaw, before_yaw, "the camera did not actually move");
        assert!(app.camera.view().to_cols_array().iter().all(|v| v.is_finite()));
    }

    /// A click on Top asks for a pitch of exactly ninety degrees, which is
    /// exactly where `look_at` used to collapse.
    ///
    /// **This replaces `flying_to_a_pole_stops_short_of_collapsing_the_view`,
    /// and the inversion is the fix.** The old test asserted the camera did
    /// NOT arrive at the pole -- because `PITCH_SAFE` stopped it 0.02 rad
    /// short, so clicking "top" gave a view that was visibly not the top. The
    /// composed basis is orthonormal there, so the assertion is now that the
    /// camera lands exactly on the pole and the matrix survives it.
    #[test]
    fn flying_to_a_pole_arrives_at_the_pole_with_the_view_intact() {
        for direction in [Vec3::Y, Vec3::NEG_Y] {
            let mut app = app();
            app.fly_to(crate::navcube::Part { direction, extremes: 1 });
            finish_flight(&mut app);
            assert!(app.flight.is_none());
            assert!(
                (app.camera.pitch.abs() - std::f32::consts::FRAC_PI_2).abs() < 1.0e-4,
                "{direction:?} did not reach the pole: pitch {}",
                app.camera.pitch
            );
            // Straight down means the view direction is world up, exactly.
            let looking = (app.camera.target - app.camera.eye()).normalize();
            assert!(
                looking.dot(-direction).abs() > 0.9999,
                "{direction:?} did not end up looking along the axis: {looking:?}"
            );
            assert!(app.camera.view().to_cols_array().iter().all(|v| v.is_finite()));
            assert!((app.camera.right().length() - 1.0).abs() < 1.0e-4);
            assert!(app.camera.right().dot(app.camera.up()).abs() < 1.0e-4);
        }
    }

    /// The camera must not spin several times round to reach a heading a few
    /// degrees away, which is what interpolating an unwrapped yaw would do.
    #[test]
    fn a_flight_takes_the_short_way_round_and_unwinds_any_roll() {
        let mut app = app();
        // Many turns of yaw, and a roll from the puck, as a session would
        // accumulate.
        app.camera.yaw = std::f32::consts::TAU * 3.0 + 0.2;
        app.camera.roll = 0.7;

        app.fly_to(crate::navcube::Part { direction: Vec3::Z, extremes: 1 });
        let flight = app.flight.expect("no flight started");
        let travel = (flight.to.0 - flight.from.0).abs();
        assert!(travel <= std::f32::consts::PI + 1.0e-4, "the flight would spin {travel} radians");

        finish_flight(&mut app);
        assert!(
            app.camera.roll.abs() < 1.0e-4,
            "a standard view should be level, roll was {}",
            app.camera.roll
        );
    }

    /// The flight is driven off the frame tick, so that link needs its own
    /// test: the tests above supply their own time to get a result at all.
    #[test]
    fn the_frame_tick_is_what_advances_a_flight() {
        let mut app = app();
        app.fly_to(crate::navcube::Part { direction: Vec3::X, extremes: 1 });
        // The first frame has no previous one to measure against, so it
        // contributes nothing; the second is the one that moves.
        update(&mut app, Message::Frame);
        update(&mut app, Message::Frame);
        let elapsed = app.flight.expect("the flight ended early").elapsed_ms;
        assert!(elapsed > 0.0, "the frame tick did not advance the flight");
    }

    fn right_press(app: &mut Hani, at: Vector) {
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Right,
            position: at,
            size: SIZE,
        });
    }

    fn right_release(app: &mut Hani) {
        app.on_pointer(PointerEvent::Released { button: PointerButton::Right });
    }

    /// Right drag already orbits, so the menu has to be told apart from it by
    /// movement and time. All three outcomes matter, and getting any of them
    /// wrong makes either the menu or the orbit unusable.
    #[test]
    fn a_right_click_opens_the_menu_but_a_right_drag_orbits() {
        let centre = centre_of_viewport();
        let moved_to = |dx: f32| Vector::new(SIZE.x / 2.0 + dx, SIZE.y / 2.0);

        // A click that does not move: the menu.
        let mut click = app();
        right_press(&mut click, centre);
        right_release(&mut click);
        assert!(click.menu.is_some(), "a right click did not open the menu");

        // A drag: orbits, and opens nothing.
        let mut orbit = app();
        let before = orbit.camera.yaw;
        right_press(&mut orbit, centre);
        orbit.on_pointer(PointerEvent::Moved { position: moved_to(60.0), size: SIZE });
        right_release(&mut orbit);
        assert!(orbit.menu.is_none(), "an orbit opened the menu");
        assert_ne!(orbit.camera.yaw, before, "the orbit did not happen");

        // A drag that returns to where it started is still a drag.
        let mut returned = app();
        right_press(&mut returned, centre);
        for dx in [40.0, 0.0] {
            returned.on_pointer(PointerEvent::Moved { position: moved_to(dx), size: SIZE });
        }
        right_release(&mut returned);
        assert!(returned.menu.is_none(), "an orbit that came back opened the menu");

        // Inside the slop is still a click: a hand never holds perfectly still.
        let mut wobble = app();
        right_press(&mut wobble, centre);
        wobble.on_pointer(PointerEvent::Moved {
            position: moved_to(CLICK_SLOP_PX * 0.5),
            size: SIZE,
        });
        right_release(&mut wobble);
        assert!(wobble.menu.is_some(), "a click that wobbled a pixel opened nothing");
    }

    #[test]
    fn the_menu_closes_on_escape_and_on_a_click_elsewhere() {
        let mut app = app();
        right_press(&mut app, centre_of_viewport());
        right_release(&mut app);
        assert!(app.menu.is_some());

        update(&mut app, Message::MenuClosed);
        assert!(app.menu.is_none(), "escape did not close it");

        // Open again, then click elsewhere: the click closes it and must not
        // also sculpt, or dismissing a menu would carve the model.
        right_press(&mut app, centre_of_viewport());
        right_release(&mut app);
        let probe = app
            .surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0))
            .expect("the centre should hit the sphere");
        let before = app.doc.active_volume().sample_world(probe);

        press(&mut app, centre_of_viewport());
        release(&mut app);
        assert!(app.menu.is_none(), "a click elsewhere did not close the menu");
        assert_eq!(
            app.doc.active_volume().sample_world(probe),
            before,
            "dismissing the menu sculpted"
        );
    }

    /// A number typed in the menu, as in the panel and the bar: the value
    /// moves on enter and not before, held to the slider's range, and junk
    /// leaves it alone. The menu's own fields applied every keystroke that
    /// parsed; they are the kit's `ui::number` now (`docs/WORKSPACES.md`).
    #[test]
    fn a_typed_radius_arrives_on_enter_and_still_clamps() {
        let mut app = app();
        let set = |app: &mut Hani, carried: Message| {
            update(app, Message::FieldSet("brush.radius", Box::new(carried)));
        };
        for typed in ["", "2", "2.", "2.5"] {
            update(&mut app, Message::FieldEdited("brush.radius", typed.to_string()));
        }
        assert_ne!(app.brush.radius, 2.5, "typing alone moved the value");
        set(&mut app, Message::BrushRadiusChanged(2.5));
        assert!((app.brush.radius - 2.5).abs() < 1.0e-5, "the value never arrived");
        assert!(app.drafts.is_empty());
        // Out of range is held to the slider's range by the field, which
        // is what sends it.
        let submitted = |app: &Hani, key: &'static str, typed: &str| {
            let mut drafts = crate::ui::Drafts::new();
            drafts.insert(key, typed.to_string());
            match key {
                "brush.radius" => crate::ui::number(
                    &drafts,
                    key,
                    "Radius",
                    app.brush.radius,
                    MIN_RADIUS_MM..=app.max_radius(),
                    Message::BrushRadiusChanged,
                )
                .submitted(),
                _ => crate::ui::number(
                    &drafts,
                    key,
                    "Strength",
                    app.brush.strength,
                    MIN_STRENGTH..=app.max_strength(),
                    Message::BrushStrengthChanged,
                )
                .submitted(),
            }
        };
        let message = submitted(&app, "brush.radius", "500");
        update(&mut app, message);
        assert_eq!(app.brush.radius, app.max_radius());
        let message = submitted(&app, "brush.strength", "-5");
        update(&mut app, message);
        assert_eq!(app.brush.strength, MIN_STRENGTH);
        // And junk leaves the value where it was.
        let message = submitted(&app, "brush.radius", "banana");
        update(&mut app, message);
        assert_eq!(app.brush.radius, app.max_radius());
    }

    #[test]
    fn hovering_the_cube_hides_the_brush_ring() {
        let mut app = app();
        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        assert!(app.hover.is_some(), "the model should be under the pointer");

        let (origin, size) = crate::navcube::corner_rect(Vec2::new(SIZE.x, SIZE.y));
        let middle = origin + size * 0.5;
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(middle.x, middle.y),
            size: SIZE,
        });

        assert!(app.cube_hover.is_some(), "the cube should be under the pointer");
        // No ring: a press here will not sculpt, and drawing one would promise
        // that it would.
        assert!(app.hover.is_none());
        assert!(app.shared.overlay_snapshot().lines.is_empty());
    }

    #[test]
    fn the_cursor_ring_appears_where_the_pointer_meets_the_model() {
        let mut app = app();
        assert!(app.hover.is_none(), "nothing has been pointed at yet");

        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        assert!(app.hover.is_some(), "the centre of the viewport should hit the sphere");
        // The geometry lives in the shared frame, not in `self.overlay`: the
        // application swaps its buffer over to the renderer and keeps the empty
        // one back, which is what stops either side allocating.
        let drawn = app.shared.overlay_snapshot();
        assert!(!drawn.lines.is_empty(), "no ring was handed to the renderer");

        // A corner of the viewport misses the sphere, so there is nothing to
        // draw and that absence is the signal a press would do nothing.
        app.on_pointer(PointerEvent::Moved { position: Vector::new(2.0, 2.0), size: SIZE });
        assert!(app.hover.is_none(), "a corner should miss the model");
        assert!(
            app.shared.overlay_snapshot().lines.is_empty(),
            "a ring was drawn for a pointer that is off the model"
        );
    }

    /// Undo has to walk the brush ring back onto the surface it restored.
    ///
    /// The ring is not a screen space circle drawn over the model: every one
    /// of its points is pushed onto the field by `cursor::onto_surface`, so
    /// the geometry the renderer is holding is only correct for the field it
    /// was built from. Undo changes the field without the pointer moving,
    /// which is the one way the two can come apart -- and before
    /// [`Hani::after_history_step`] refreshed the overlay, ctrl+Z left the
    /// ring standing on the surface the stroke had made, floating off the
    /// model until some later pointer motion happened to rebuild it.
    ///
    /// **Measured rather than argued.** With the default sphere and a
    /// full strength Draw at the centre of the viewport, the stale ring sits
    /// 0.49 voxels off the restored field and a refreshed one sits 0.0016
    /// voxels off it, against the 0.1 voxel tolerance `onto_surface` stops
    /// walking at -- five times the tolerance apart in one direction and
    /// sixty times inside it in the other. The first assertion is there
    /// because a fixture whose stroke did not actually move the surface under
    /// the ring would pass the second one while testing nothing.
    ///
    /// Deleting the `refresh_overlay` call from `after_history_step` fails
    /// this test and, as of writing, nothing else in the workspace.
    #[test]
    fn undoing_a_stroke_walks_the_brush_ring_back_onto_the_surface_it_restores() {
        // What `cursor::onto_surface` treats as "on the surface", in voxels.
        const ON_SURFACE: f32 = 0.1;

        let ring = |app: &Hani| -> Vec<Vec3> {
            app.shared
                .overlay_snapshot()
                .lines
                .iter()
                .map(|vertex| Vec3::from_array(vertex.position))
                .collect()
        };
        let worst_off_surface = |app: &Hani, points: &[Vec3]| -> f32 {
            points
                .iter()
                .map(|point| app.doc.active_volume().sample_world(*point).abs())
                .fold(0.0f32, f32::max)
        };

        let mut app = app();
        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        assert!(app.hover.is_some(), "the centre of the viewport should hit the sphere");

        // Deep enough to move the surface the ring is standing on. The pointer
        // does not move again after this, so nothing but undo can rebuild the
        // overlay.
        app.brush.strength = 1.0;
        press(&mut app, centre_of_viewport());
        release(&mut app);

        let stale = ring(&app);
        assert!(!stale.is_empty(), "no ring was handed to the renderer");

        update(&mut app, Message::Undo);

        let stale_error = worst_off_surface(&app, &stale);
        assert!(
            stale_error > ON_SURFACE,
            "the fixture asserts nothing: the stroke left the ring only {stale_error} voxels \
             off the surface undo restored, which a stale ring would pass"
        );

        let error = worst_off_surface(&app, &ring(&app));
        assert!(
            error <= ON_SURFACE,
            "undo left the brush ring {error} voxels off the surface it restored, so the \
             overlay was never rebuilt"
        );
    }

    #[test]
    fn the_radius_keys_stay_inside_the_range_the_slider_offers() {
        let mut app = app();
        for _ in 0..200 {
            update(&mut app, Message::BrushRadiusScaled(1.5));
        }
        assert_eq!(app.brush.radius, MAX_RADIUS_MM);
        for _ in 0..200 {
            update(&mut app, Message::BrushRadiusScaled(1.0 / 1.5));
        }
        assert_eq!(app.brush.radius, MIN_RADIUS_MM);
    }

    /// ZBrush and Nomad both invert on alt; this had control first. Both work,
    /// and holding both is still one inversion rather than two.
    #[test]
    fn either_modifier_inverts_and_holding_both_is_not_a_double_negative() {
        let mut app = app();
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        assert_eq!(app.stroke_direction(), BrushDirection::Add);

        let modifiers = |app: &mut Hani, control, alt| {
            app.on_pointer(PointerEvent::Modifiers { shift: false, control, alt });
        };

        modifiers(&mut app, true, false);
        assert_eq!(app.stroke_direction(), BrushDirection::Subtract, "control should invert");

        modifiers(&mut app, false, true);
        assert_eq!(app.stroke_direction(), BrushDirection::Subtract, "alt should invert");

        modifiers(&mut app, true, true);
        assert_eq!(
            app.stroke_direction(),
            BrushDirection::Subtract,
            "both held is one inversion, not two"
        );

        modifiers(&mut app, false, false);
        assert_eq!(app.stroke_direction(), BrushDirection::Add, "releasing should restore");
    }

    /// The eraser end still combines with a modifier the way it always did:
    /// inverting an inverted stroke gives back the additive brush.
    #[test]
    fn alt_combines_with_the_eraser_the_way_control_does() {
        for (control, alt) in [(true, false), (false, true)] {
            let mut app = app();
            update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
            app.tablet.simulate(pen(glam::Vec2::ZERO, true));
            assert_eq!(app.stroke_direction(), BrushDirection::Subtract, "the eraser alone");

            app.on_pointer(PointerEvent::Modifiers { shift: false, control, alt });
            assert_eq!(
                app.stroke_direction(),
                BrushDirection::Add,
                "a modifier over the eraser should give back the additive brush"
            );
        }
    }

    #[test]
    fn control_does_not_invert_a_brush_that_has_no_opposite() {
        let mut app = app();
        app.control = true;

        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        assert_eq!(app.stroke_direction(), BrushDirection::Subtract);

        update(&mut app, Message::BrushKindChanged(BrushKind::Smooth));
        assert_eq!(
            app.stroke_direction(),
            BrushDirection::Add,
            "smooth has no opposite, so inverting it should do nothing"
        );
    }

    fn pen(tilt: glam::Vec2, eraser: bool) -> PenState {
        PenState { in_proximity: true, pressure: 1.0, eraser, tilt }
    }

    #[test]
    fn the_eraser_end_inverts_the_brush() {
        let mut app = app();
        assert_eq!(app.stroke_direction(), BrushDirection::Add);

        app.tablet.simulate(pen(glam::Vec2::ZERO, true));
        assert_eq!(app.stroke_direction(), BrushDirection::Subtract);

        // The modifier and the eraser combine rather than override, so holding
        // one while using the other gives back the additive brush.
        app.control = true;
        assert_eq!(app.stroke_direction(), BrushDirection::Add);
    }

    #[test]
    fn the_eraser_does_nothing_to_a_brush_with_no_opposite() {
        let mut app = app();
        update(&mut app, Message::BrushKindChanged(BrushKind::Smooth));
        app.tablet.simulate(pen(glam::Vec2::ZERO, true));
        assert_eq!(app.stroke_direction(), BrushDirection::Add);
    }

    #[test]
    fn a_pen_that_is_away_cannot_erase() {
        let app = app();
        app.tablet.simulate(PenState { eraser: true, ..PenState::NONE });
        assert_eq!(app.stroke_direction(), BrushDirection::Add);
    }

    #[test]
    fn leaning_the_pen_produces_a_world_space_lean_along_the_camera_axes() {
        let mut app = app();
        assert_eq!(app.pen_lean(), Vec3::ZERO, "no pen means no lean");

        app.tablet.simulate(pen(glam::Vec2::new(1.0, 0.0), false));
        let lean = app.pen_lean();
        assert!(
            (lean.length() - MAX_TILT).abs() < 1.0e-4,
            "a fully tilted pen should lean by the maximum angle, got {}",
            lean.length()
        );
        assert!(
            lean.normalize().dot(app.camera.right()) > 0.999,
            "tilt on the x axis should lean along the camera's right axis"
        );

        // Tilt on the y axis leans down the screen, which is away from up.
        app.tablet.simulate(pen(glam::Vec2::new(0.0, 1.0), false));
        assert!(app.pen_lean().normalize().dot(app.camera.up()) < -0.999);

        update(&mut app, Message::TiltToggled(false));
        assert_eq!(app.pen_lean(), Vec3::ZERO, "turning tilt off must disable it entirely");
    }

    #[test]
    fn leaning_the_pen_moves_where_the_clay_lands() {
        // The end to end statement of what tilt is for: the same stroke on the
        // same spot puts material somewhere else when the pen is leaned.
        //
        // Measured as the difference between the two sides of the stroke rather
        // than against an upright stroke. Leaning also reduces how far the
        // brush pushes outward, by the cosine of the angle, and that term is
        // larger than the sideways one at this scale. Comparing left against
        // right cancels it, because it applies to both equally.
        let sculpt = |tilt: glam::Vec2| {
            let mut app = app();
            app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
            app.brush.strength = 0.8;
            app.brush.radius = 6.0;
            app.tablet.simulate(pen(tilt, false));

            let hit = app
                .surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0))
                .expect("the centre of the view is on the model");
            let sideways = app.camera.right() * 4.0;

            // One stamp moves the surface by a fraction of a voxel, so the
            // stroke has to be laid down repeatedly to be measurable.
            for _ in 0..8 {
                press(&mut app, centre_of_viewport());
                release(&mut app);
            }
            (
                app.doc.active_volume().sample_world(hit + sideways),
                app.doc.active_volume().sample_world(hit - sideways),
            )
        };

        let (right_upright, left_upright) = sculpt(glam::Vec2::ZERO);
        assert!(
            (right_upright - left_upright).abs() < 0.02,
            "an upright pen should build up evenly on both sides: \
             {right_upright} against {left_upright}"
        );

        let (right_leaned, left_leaned) = sculpt(glam::Vec2::new(1.0, 0.0));
        assert!(
            right_leaned < left_leaned - 0.02,
            "leaning right should pile material to the right: \
             right {right_leaned}, left {left_leaned}"
        );

        let (right_other_way, left_other_way) = sculpt(glam::Vec2::new(-1.0, 0.0));
        assert!(
            left_other_way < right_other_way - 0.02,
            "leaning left should pile material to the left: \
             right {right_other_way}, left {left_other_way}"
        );
    }

    #[test]
    fn resetting_discards_history_that_refers_to_the_old_model() {
        // Undoing into a volume the entry was not recorded against would splice
        // pieces of the discarded model back in.
        let mut app = app();
        press(&mut app, centre_of_viewport());
        release(&mut app);
        assert!(app.history.can_undo());

        // Reset now goes through the unsaved-work prompt, because it discards
        // the document. Answering Discard is what the button used to do on its
        // own; the history assertions below are unchanged.
        update(&mut app, Message::ResetSphere);
        update(&mut app, Message::ConfirmAnswered(ConfirmChoice::Discard));

        assert!(!app.history.can_undo(), "reset must clear history");
        assert!(!app.history.can_redo());
    }

    #[test]
    fn every_brush_can_be_driven_from_the_interface_without_panicking() {
        // Cheap breadth: each brush goes through the whole application path
        // once, which is where a bad plane or a zero normal would surface.
        for kind in BrushKind::ALL {
            let mut app = app();
            update(&mut app, Message::BrushKindChanged(kind));
            update(&mut app, Message::BrushStrengthChanged(0.6));
            press(&mut app, centre_of_viewport());
            app.on_pointer(PointerEvent::Moved {
                position: centre_of_viewport() + Vector::new(30.0, 12.0),
                size: SIZE,
            });
            release(&mut app);
            assert_eq!(app.history_stats.undo_entries, 1, "{kind} recorded no undo entry");
        }
    }

    // --- re-orienting the model ------------------------------------------

    /// The middle of the navigation cube, which always picks a face rather than
    /// an edge or a corner.
    fn cube_centre() -> Vector {
        let (origin, size) = crate::navcube::corner_rect(Vec2::new(SIZE.x, SIZE.y));
        let middle = origin + size * 0.5;
        Vector::new(middle.x, middle.y)
    }

    /// A model with material in one identifiable place, so a turn is visible.
    ///
    /// The seeded sphere is centred and symmetric, so every rotation leaves it
    /// looking exactly the same -- a test built on it would pass whether or not
    /// anything turned.
    fn lopsided(app: &mut Hani) {
        let mut volume = hani_core::Volume::new(app.doc.voxel_size());
        volume.seed_sphere(Vec3::new(0.0, 30.0, 0.0), 8.0);
        volume.mark_everything_dirty();
        app.doc.replace_active_volume(volume);
        app.remesh_dirty();
    }

    /// The collision this feature had to resolve. A right press on the cube
    /// used to fall through to an orbit and then, on release, open the BRUSH
    /// menu on top of the cube -- so the one gesture that should have asked
    /// about orientation asked about brush radius instead.
    #[test]
    fn a_right_click_on_the_cube_opens_the_cubes_menu_and_not_the_brushs() {
        let mut app = app();
        right_press(&mut app, cube_centre());
        right_release(&mut app);

        assert!(app.cube_menu.is_some(), "the cube's own menu did not open");
        assert!(app.menu.is_none(), "the brush menu opened on top of the cube");
        assert!(app.drag.is_none(), "the press started an orbit");
        assert!(app.flight.is_none(), "a right click flew the camera as though it were a left one");
    }

    #[test]
    fn a_left_click_on_the_cube_still_flies_the_camera() {
        // The half of the cube's behaviour that already existed, pinned
        // alongside the new half so a change to the routing cannot take it out
        // silently.
        let mut app = app();
        press(&mut app, cube_centre());
        release(&mut app);

        assert!(app.flight.is_some(), "the left click stopped moving the camera");
        assert!(app.cube_menu.is_none(), "a left click opened the orientation menu");
    }

    #[test]
    fn the_next_press_closes_the_cube_menu_without_sculpting() {
        let mut app = app();
        let probe = app.camera.eye().normalize() * MODEL_RADIUS_MM;
        right_press(&mut app, cube_centre());
        right_release(&mut app);
        assert!(app.cube_menu.is_some());

        let before = app.doc.active_volume().sample_world(probe);
        press(&mut app, centre_of_viewport());
        release(&mut app);

        assert!(app.cube_menu.is_none(), "the menu stayed open");
        assert_eq!(
            app.doc.active_volume().sample_world(probe),
            before,
            "dismissing the menu also sculpted"
        );
    }

    #[test]
    fn escape_closes_the_cube_menu() {
        let mut app = app();
        right_press(&mut app, cube_centre());
        right_release(&mut app);
        update(&mut app, Message::MenuClosed);
        assert!(app.cube_menu.is_none());
    }

    #[test]
    fn choosing_a_direction_turns_the_model_and_leaves_the_camera_alone() {
        let mut app = app();
        lopsided(&mut app);
        let (yaw, pitch, roll) = (app.camera.yaw, app.camera.pitch, app.camera.roll);
        let above = Vec3::new(0.0, 30.0, 0.0);
        let in_front = Vec3::new(0.0, 0.0, 30.0);
        assert!(app.doc.active_volume().sample_world(above) < 0.0, "the fixture is wrong");

        app.cube_menu =
            Some(CubeMenu { at: Vec2::new(700.0, 20.0), facing: hani_core::Facing::Up });
        update(&mut app, Message::OrientFace(hani_core::Facing::Front));

        // The model moved...
        assert!(
            app.doc.active_volume().sample_world(in_front) < 0.0,
            "the material did not arrive in front of the origin"
        );
        assert!(
            app.doc.active_volume().sample_world(above) >= 0.0,
            "the material is still above the origin"
        );
        // ...and the camera did not. This is the whole difference between
        // turning the model and re-aiming the view, and it is what makes an
        // export land upright.
        assert_eq!((app.camera.yaw, app.camera.pitch, app.camera.roll), (yaw, pitch, roll));

        assert!(app.cube_menu.is_none(), "the menu stayed open over a model that had moved");
        assert!(app.unsaved, "a turned model is unsaved work");
        assert!(!app.history.can_undo(), "history outlived the volume it named bricks of");
        assert!(app.perf.dirty_bricks > 0, "nothing was remeshed, so the turn is invisible");
    }

    #[test]
    fn turning_a_face_onto_itself_changes_nothing() {
        // The menu greys this one out, but the message is public and the guard
        // is in `orient`, not in the widget.
        let mut app = app();
        lopsided(&mut app);
        app.unsaved = false;
        let above = Vec3::new(0.0, 30.0, 0.0);
        let before = app.doc.active_volume().sample_world(above);

        app.cube_menu = Some(CubeMenu { at: Vec2::ZERO, facing: hani_core::Facing::Up });
        update(&mut app, Message::OrientFace(hani_core::Facing::Up));

        assert_eq!(app.doc.active_volume().sample_world(above), before);
        assert!(!app.unsaved, "a turn that did nothing still marked the document dirty");
    }

    #[test]
    fn turning_back_returns_the_model() {
        // The recovery path, and the reason there is no undo entry: a quarter
        // turn is exact, so the other way round is a real undo rather than an
        // approximation of one.
        let mut app = app();
        lopsided(&mut app);
        let above = Vec3::new(0.0, 30.0, 0.0);
        let before = app.doc.active_volume().sample_world(above);

        app.cube_menu = Some(CubeMenu { at: Vec2::ZERO, facing: hani_core::Facing::Up });
        update(&mut app, Message::OrientFace(hani_core::Facing::Front));
        app.cube_menu = Some(CubeMenu { at: Vec2::ZERO, facing: hani_core::Facing::Front });
        update(&mut app, Message::OrientFace(hani_core::Facing::Up));

        assert_eq!(
            app.doc.active_volume().sample_world(above),
            before,
            "turning back did not return the model"
        );
    }

    /// The same import, with its sphere where the camera is looking.
    ///
    /// `adopt_import` frames the ORIGIN at `MODEL_RADIUS_MM`, and
    /// `imported_with`'s sphere is a 6 mm ball twenty millimetres above it: a
    /// ray through the centre of the viewport misses it completely. A test
    /// that presses there to prove the press was STOPPED therefore proves
    /// nothing, which is exactly what
    /// `a_press_behind_the_orientation_prompt_does_not_sculpt` was doing --
    /// verified by removing the guard and watching it pass.
    fn imported_under_the_cursor(
        resting_up: Option<hani_core::Facing>,
    ) -> crate::message::Imported {
        let mut volume = hani_core::Volume::new(0.5);
        volume.seed_sphere(Vec3::ZERO, MODEL_RADIUS_MM);
        volume.mark_everything_dirty();
        crate::message::Imported { volume, ..imported_with(resting_up) }
    }

    /// Build what a finished import delivers, with a chosen guess about which
    /// way the mesh's own up pointed.
    fn imported_with(resting_up: Option<hani_core::Facing>) -> crate::message::Imported {
        let mut volume = hani_core::Volume::new(0.5);
        volume.seed_sphere(Vec3::new(0.0, 20.0, 0.0), 6.0);
        volume.mark_everything_dirty();
        crate::message::Imported {
            volume,
            report: hani_core::voxelise::VoxeliseReport::default(),
            source: std::path::PathBuf::from("nightwing.obj"),
            elapsed_ms: 0.0,
            resting_up,
        }
    }

    #[test]
    fn an_import_that_came_in_lying_down_raises_the_prompt() {
        let mut app = app();
        app.adopt_import(imported_with(Some(hani_core::Facing::Back)));
        assert_eq!(
            app.orient_prompt,
            Some(hani_core::Facing::Back),
            "the model is on its back and nothing offered to stand it up"
        );
    }

    #[test]
    fn an_import_that_is_already_upright_is_not_asked_about() {
        // The case that decides whether this feature is usable or a nuisance:
        // an STL exported for a slicer sits on the bed and arrives standing, so
        // the guess resolves to `Up` and must raise nothing at all. Getting
        // this wrong would put a dialog in front of nearly every print file.
        let mut app = app();
        app.adopt_import(imported_with(Some(hani_core::Facing::Up)));
        assert!(app.orient_prompt.is_none(), "an upright model was asked about");
    }

    #[test]
    fn an_import_with_no_tell_is_not_guessed_at() {
        let mut app = app();
        app.adopt_import(imported_with(None));
        assert!(app.orient_prompt.is_none());
    }

    #[test]
    fn accepting_the_prompt_stands_the_model_up() {
        let mut app = app();
        app.adopt_import(imported_with(Some(hani_core::Facing::Back)));
        // The fixture's material is above the origin; a model whose own up
        // points backwards has to be turned for that to mean anything, so
        // check where it lands rather than that it merely moved.
        let turned =
            hani_core::AxisRotation::taking(hani_core::Facing::Back, hani_core::Facing::Up)
                .apply(Vec3::new(0.0, 20.0, 0.0));

        update(&mut app, Message::OrientPromptAnswered(true));

        assert!(app.orient_prompt.is_none(), "the prompt stayed up after being answered");
        assert!(
            app.doc.active_volume().sample_world(turned) < 0.0,
            "the model did not turn the way promised"
        );
    }

    #[test]
    fn declining_the_prompt_leaves_the_model_exactly_as_imported() {
        let mut app = app();
        app.adopt_import(imported_with(Some(hani_core::Facing::Back)));
        let above = Vec3::new(0.0, 20.0, 0.0);
        let before = app.doc.active_volume().sample_world(above);

        update(&mut app, Message::OrientPromptAnswered(false));

        assert!(app.orient_prompt.is_none());
        assert_eq!(
            app.doc.active_volume().sample_world(above),
            before,
            "declining still turned the model"
        );
    }

    #[test]
    fn a_press_behind_the_orientation_prompt_does_not_sculpt() {
        // iced 0.14's `stack!` layers do not block what is underneath them, so
        // the scrim is the widget that swallows presses and this early return
        // is the guarantee behind it. Without both, a click behind the card
        // carves the model the user is being asked about.
        let mut app = app();
        app.adopt_import(imported_under_the_cursor(Some(hani_core::Facing::Back)));
        assert!(app.orient_prompt.is_some());

        let probe = app.camera.eye().normalize() * MODEL_RADIUS_MM;
        let before = app.doc.active_volume().sample_world(probe);
        let entries = app.history_stats.undo_entries;
        press(&mut app, centre_of_viewport());
        release(&mut app);

        assert_eq!(
            app.doc.active_volume().sample_world(probe),
            before,
            "a press reached the model behind it"
        );
        // The probe above is a fixed point on the ORIGINAL sphere, and an
        // imported model need not have surface there -- with the guard removed
        // by hand this assertion still passed, so it is not the one doing the
        // work. A stroke that lands anywhere at all records an undo entry.
        assert_eq!(
            app.history_stats.undo_entries, entries,
            "it recorded a stroke behind the prompt"
        );
        assert!(app.drag.is_none());
    }

    #[test]
    fn escape_declines_the_orientation_prompt() {
        let mut app = app();
        app.adopt_import(imported_with(Some(hani_core::Facing::Back)));
        let above = Vec3::new(0.0, 20.0, 0.0);
        let before = app.doc.active_volume().sample_world(above);

        update(&mut app, Message::MenuClosed);

        assert!(app.orient_prompt.is_none(), "escape did not dismiss it");
        assert_eq!(app.doc.active_volume().sample_world(above), before, "escape turned the model");
    }
}

#[cfg(test)]
mod working_size_tests {
    use super::*;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// Scaling is a change to `voxel_size` and nothing else, so the field has
    /// to come through it untouched. If this ever fails, the operation stopped
    /// being free and started resampling.
    #[test]
    fn resizing_scales_the_model_without_touching_a_voxel() {
        let mut app = app();
        let before: Vec<_> = app
            .doc
            .active_volume()
            .brick_coords()
            .map(|c| (c, app.doc.active_volume().sample_voxel(c.origin())))
            .collect();
        let was =
            app.doc.active_volume().surface_bounds().expect("the starting ball has a surface");
        let longest_before = (was.1 - was.0).max_element();
        let voxel_before = app.doc.voxel_size();

        app.working_size_field = "30".into();
        update(&mut app, Message::WorkingSizeCommitted);

        let now = app.doc.active_volume().surface_bounds().expect("still a surface");
        let longest_after = (now.1 - now.0).max_element();
        assert!(
            (longest_after - 30.0).abs() < 0.5,
            "asked for 30 mm and got {longest_after:.2} mm (from {longest_before:.2})"
        );

        let factor = 30.0 / longest_before;
        assert!(
            (app.doc.voxel_size() - voxel_before * factor).abs() < 1.0e-6,
            "the voxel size should have scaled with the model"
        );
        for (coord, value) in before {
            assert_eq!(
                app.doc.active_volume().sample_voxel(coord.origin()),
                value,
                "a voxel changed, so this resampled instead of rescaling"
            );
        }
    }

    /// The detail did NOT improve, and the interface must not imply it did.
    #[test]
    fn resizing_buys_no_extra_detail() {
        let mut app = app();
        let bounds = app.doc.active_volume().surface_bounds().expect("a surface");
        let across_before =
            ((bounds.1 - bounds.0).max_element() / app.doc.voxel_size()).round() as i64;

        app.working_size_field = "12".into();
        update(&mut app, Message::WorkingSizeCommitted);

        let bounds = app.doc.active_volume().surface_bounds().expect("a surface");
        let across_after =
            ((bounds.1 - bounds.0).max_element() / app.doc.voxel_size()).round() as i64;
        assert_eq!(
            across_before, across_after,
            "the model has the same number of voxels across it, whatever size it is"
        );
    }

    /// A size that would push the voxel outside the range the finer and coarser
    /// buttons work in has to be refused, or the model lands somewhere those
    /// buttons can never bring it back from.
    #[test]
    fn a_size_that_would_put_the_voxel_out_of_range_is_refused() {
        let mut app = app();
        let voxel_before = app.doc.voxel_size();

        app.working_size_field = "0.001".into();
        update(&mut app, Message::WorkingSizeCommitted);

        assert_eq!(app.doc.voxel_size(), voxel_before, "the model was resized anyway");
        assert!(app.status.contains("could not resize"), "no refusal shown: {}", app.status);
    }

    #[test]
    fn nonsense_in_the_field_is_refused_rather_than_parsed_as_zero() {
        let mut app = app();
        let voxel_before = app.doc.voxel_size();
        app.working_size_field = "big".into();
        update(&mut app, Message::WorkingSizeCommitted);
        assert_eq!(app.doc.voxel_size(), voxel_before);
        assert!(app.status.contains("could not resize"), "{}", app.status);
    }

    /// The radius ceiling at the default voxel size must be **exactly what it
    /// always was**. The voxel rule is there to stop a fine lattice reaching an
    /// unusable brush, not to change the tool anyone is using today.
    #[test]
    fn the_voxel_rule_does_not_touch_the_brush_at_the_default_size() {
        let app = app();
        assert_eq!(app.doc.voxel_size(), VOXEL_SIZE_MM);
        assert_eq!(
            app.max_radius(),
            MAX_RADIUS_MM,
            "100 voxels is 25 mm at the default, above the millimetre ceiling, so nothing moves"
        );
    }

    /// And at a resin lattice it must bite, because the millimetre ceiling
    /// there is 640 voxels of radius -- measured at roughly a quarter of a
    /// second per stamp.
    ///
    /// The lattice is reached with [`Document::rescale`], which multiplies
    /// `voxel_size` and touches no voxel, rather than with `resample`, which
    /// rebuilds the default sphere's narrow band eight times finer.
    /// `max_radius` reads nothing but `doc.voxel_size()`, and `0.25 * 0.125`
    /// is `0.03125` to the bit, so the resample bought this assertion nothing
    /// and was refused rather than merely not chosen: measured on the built
    /// test binary it cost 4.6 s and 2.4 GB of peak RSS for one test, against
    /// 0.08 s and 75 MB through `rescale`. Cargo runs test binaries and their
    /// threads in parallel, so a 2.4 GB spike here is a spike beside the GPU
    /// offscreen suite on a CI runner. If a later change gives `max_radius` a
    /// reason to read actual voxels, this has to become a real resample --
    /// and then say so here.
    #[test]
    fn the_voxel_rule_caps_the_brush_at_a_resin_lattice() {
        let mut app = app();
        app.doc.rescale(0.125);
        assert!(
            (app.max_radius() - 3.125).abs() < 1.0e-6,
            "expected 100 voxels of radius, got {} mm",
            app.max_radius()
        );

        // And a request past it is actually clamped, not merely reported.
        drop(app.update(Message::BrushRadiusChanged(MAX_RADIUS_MM)));
        assert!(
            app.brush.radius <= app.max_radius() + 1.0e-6,
            "the slider handed out {} mm past a {} mm ceiling",
            app.brush.radius,
            app.max_radius()
        );
    }

    /// A finer step the pool cannot hold lands on the finest that fits,
    /// rather than refusing with directions to a rung the buttons cannot
    /// reach. The refusal is kept only for when there is nowhere finer left.
    #[test]
    fn a_step_that_does_not_fit_lands_on_the_finest_that_does() {
        let mut app = app();
        // A pool measured at 6M of 11M vertices: one halving would predict
        // 24M, far over, but there is real room above the current size.
        app.shared.set_stats_for_tests(hani_gpu::PoolStats {
            vertices_reserved: 6_000_000,
            indices_reserved: 33_000_000,
            vertex_capacity: 11_000_000,
            index_capacity: 66_000_000,
            ..Default::default()
        });

        update(&mut app, Message::Resample(VOXEL_SIZE_MM / 2.0));

        let expected = VOXEL_SIZE_MM / (11.0f32 / 6.0).sqrt() * 1.03;
        assert!(
            (app.doc.voxel_size() - expected).abs() < 1.0e-3,
            "expected to land near {expected:.3} mm, landed at {:.3}",
            app.doc.voxel_size()
        );
        assert!(
            app.status.contains("finest the mesh pool holds"),
            "the status must say the step was capped: {}",
            app.status
        );
    }

    /// Memory is checked before the pool, because since the pool grew itself
    /// buffers it is no longer the tighter of the two. A step the pool would
    /// happily hold can still be one that walks the machine into swap.
    #[test]
    fn a_step_that_would_exhaust_memory_is_capped_even_when_the_pool_has_room() {
        let mut app = app();
        // Plenty of pool...
        app.shared.set_stats_for_tests(hani_gpu::PoolStats {
            vertices_reserved: 1_000_000,
            indices_reserved: 6_000_000,
            vertex_capacity: hani_gpu::TOTAL_VERTEX_CAPACITY,
            index_capacity: hani_gpu::TOTAL_INDEX_CAPACITY,
            ..Default::default()
        });
        // ...but the volume is already 4 GB, so one halving would want 16.
        app.doc_stats.resident_bytes = 4 * 1024 * 1024 * 1024;
        let before = app.doc.voxel_size();

        update(&mut app, Message::Resample(before / 2.0));

        assert!(app.doc.voxel_size() < before, "it should still go finer, just not that fine");
        assert!(app.doc.voxel_size() > before / 2.0, "and not all the way to the requested size");
        assert!(
            app.status.contains("finest the mesh pool holds"),
            "the cap should be reported: {}",
            app.status
        );
    }

    /// And when the current size already sits at the fit limit, the refusal
    /// stays a refusal -- there is nowhere finer to land.
    #[test]
    fn at_the_fit_limit_the_refusal_remains() {
        let mut app = app();
        app.shared.set_stats_for_tests(hani_gpu::PoolStats {
            vertices_reserved: 10_800_000,
            indices_reserved: 64_000_000,
            vertex_capacity: 11_000_000,
            index_capacity: 66_000_000,
            ..Default::default()
        });
        let before = app.doc.voxel_size();

        update(&mut app, Message::Resample(before / 2.0));

        assert_eq!(app.doc.voxel_size(), before, "there was no room, so nothing should move");
        assert!(app.status.contains("could not resample"), "{}", app.status);
    }

    /// The readout has to name both numbers, because either alone is the
    /// scaling changed no detail, and voxels-across without millimetres cannot
    /// answer "is this enough for my printer".
    #[test]
    fn the_advice_names_both_the_resolution_and_what_it_measures() {
        let app = app();
        let advice = app.measure_detail_advice();
        assert!(advice.contains("voxels wide"), "no voxel count: {advice}");
        assert!(advice.contains("mm across"), "no physical size: {advice}");
        assert!(advice.contains("voxel "), "no voxel size: {advice}");
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;

    /// `update` returns a `Task` now. Tests do not run the iced runtime, so
    /// there is nothing to hand it to and dropping it is correct — but it must
    /// be dropped deliberately rather than by `#[allow]`, or a test that should
    /// have driven a dialog would pass silently.
    fn update(app: &mut Hani, message: Message) {
        let task = app.update(message);
        drop(task);
    }

    use hani_core::BrickCoord;

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// The starting ball has to straddle every mirror plane equally, or turning
    /// on symmetry works against a model that was never centred.
    ///
    /// It always has -- but "I measured it once" is not a guarantee, and the
    /// defaults it depends on (`MODEL_RADIUS_MM`, `VOXEL_SIZE_MM`, and the fact
    /// that voxel index 0 sits exactly on the origin) are three separate things
    /// a future change could move. This is what makes it permanent.
    #[test]
    fn the_diagnostics_carry_what_a_bug_report_needs() {
        let app = app();
        let report = app.diagnostics();
        for expected in ["HaniSculpt", "model:", "view:", "tablet:", "spacemouse:"] {
            assert!(report.contains(expected), "diagnostics are missing {expected}:\n{report}");
        }
        // The platform line is one or the other, never neither. `session:`
        // answers "Wayland or X11, and which compositor", which only Linux can
        // be asked; everywhere else the OS version is the useful fact and
        // `os:` carries it. Asserting the pair rather than one name is what
        // stops a future platform silently reporting no platform at all --
        // this test demanded `session:` on every OS and passed for a year
        // because only one OS was built.
        let platform = if cfg!(target_os = "linux") { "session:" } else { "os:" };
        assert!(
            report.contains(platform),
            "diagnostics name no platform; expected {platform}:\n{report}"
        );
        // The commit is what ties a binary to its source, which is an AGPL
        // obligation rather than a nicety.
        assert!(report.contains(build_commit()));
    }

    #[test]
    fn the_default_model_is_centred_on_every_mirror_plane() {
        let app = app();
        let (mesh, _) = app.doc.active_volume().export_mesh();
        assert!(!mesh.positions.is_empty());

        let mut low = Vec3::splat(f32::MAX);
        let mut high = Vec3::splat(f32::MIN);
        for position in &mesh.positions {
            low = low.min(*position);
            high = high.max(*position);
        }

        // Well under a voxel: the lattice is symmetric about the origin, so the
        // only error available is f32 rounding.
        let tolerance = VOXEL_SIZE_MM * 0.01;
        let midpoint = (low + high) * 0.5;
        for (axis, offset) in [("x", midpoint.x), ("y", midpoint.y), ("z", midpoint.z)] {
            assert!(offset.abs() < tolerance, "the model is off centre on {axis} by {offset} mm");
        }

        // And it reaches the same distance either way along each axis, which is
        // what "equal sides of the centreline" actually means.
        for (axis, lo, hi) in [("x", low.x, high.x), ("y", low.y, high.y), ("z", low.z, high.z)] {
            assert!(
                (hi + lo).abs() < tolerance,
                "the model reaches {hi} one way on {axis} and {lo} the other"
            );
        }

        // The field itself, in mirrored pairs, not just the mesh it produced.
        for step in 0..24 {
            let probe = Vec3::new(
                MODEL_RADIUS_MM - 1.0 + step as f32 * 0.05,
                step as f32 * 0.37,
                step as f32 * -0.21,
            );
            for flip in
                [Vec3::new(-1.0, 1.0, 1.0), Vec3::new(1.0, -1.0, 1.0), Vec3::new(1.0, 1.0, -1.0)]
            {
                let here = app.doc.active_volume().sample_world(probe);
                let mirrored = app.doc.active_volume().sample_world(probe * flip);
                assert!(
                    (here - mirrored).abs() < 1.0e-4,
                    "the field disagrees across {flip:?}: {here} against {mirrored}"
                );
            }
        }
    }

    #[test]
    fn the_default_model_exports_watertight() {
        // The most basic promise of a printing tool: what it opens with can be
        // printed.
        let app = app();
        let (_, report) = app.doc.active_volume().export_mesh();
        assert!(
            report.is_printable(),
            "the model the application starts with does not print: {}",
            report.summary()
        );
    }

    #[test]
    fn a_sculpted_model_exports_watertight() {
        let mut app = app();
        app.viewport_size = Vec2::new(800.0, 600.0);
        app.brush.strength = 0.7;
        for offset in 0..6 {
            app.on_pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: iced::Vector::new(400.0 + offset as f32 * 8.0, 300.0),
                size: iced::Vector::new(800.0, 600.0),
            });
            app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
        }

        let (_, report) = app.doc.active_volume().export_mesh();
        assert!(report.is_printable(), "{}", report.summary());
    }

    #[test]
    fn resampling_finer_increases_detail_and_keeps_it_printable() {
        let mut app = app();
        let (_, before) = app.doc.active_volume().export_mesh();
        let coarse_voxel = app.doc.voxel_size();

        update(&mut app, Message::Resample(coarse_voxel / 2.0));

        assert!((app.doc.voxel_size() - coarse_voxel / 2.0).abs() < 1.0e-6);
        let (_, after) = app.doc.active_volume().export_mesh();
        assert!(
            after.triangles > before.triangles * 2,
            "finer voxels should give far more triangles: {} against {}",
            after.triangles,
            before.triangles
        );
        assert!(after.is_printable(), "{}", after.summary());
        assert!(app.status.contains("resampled"), "the interface should say what happened");
    }

    #[test]
    fn resampling_will_not_go_past_the_limits() {
        // Memory grows as the inverse cube of the voxel size, so the bounds stop
        // a click walking the model past what the mesh pool has been measured to
        // hold. Checked on the clamp rather than by resampling, because actually
        // building a model at the limit is a gigabyte of work.
        assert_eq!(Hani::clamped_voxel_size(0.0000001), FINEST_VOXEL_MM);
        assert_eq!(Hani::clamped_voxel_size(1000.0), COARSEST_VOXEL_MM);
        assert_eq!(Hani::clamped_voxel_size(0.25), 0.25);

        // And the interface offers a step that lands inside the limits. Three
        // halvings from the default are allowed and four are not: 0.25 halved
        // three times is 0.03125, which is the finest rung of the ladder and
        // the one resin work needs. It was two halvings until 2026-08-21, when
        // the floor came down from 0.06 to 0.03 for exactly that rung.
        const _: () = assert!(FINEST_VOXEL_MM > VOXEL_SIZE_MM / 16.0);
        const _: () = assert!(FINEST_VOXEL_MM < VOXEL_SIZE_MM / 4.0);
    }

    #[test]
    fn resampling_clears_history_and_schedules_a_full_remesh() {
        let mut app = app();
        app.viewport_size = Vec2::new(800.0, 600.0);
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: iced::Vector::new(400.0, 300.0),
            size: iced::Vector::new(800.0, 600.0),
        });
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
        assert!(app.history.can_undo());

        let old_coords: Vec<BrickCoord> = app.doc.active_volume().brick_coords().collect();
        let finer = app.doc.voxel_size() / 2.0;
        update(&mut app, Message::Resample(finer));

        assert!(
            !app.history.can_undo(),
            "history refers to bricks at the old resolution and must be dropped"
        );
        assert!(app.perf.dirty_bricks > old_coords.len(), "the whole model has to be remeshed");
    }

    #[test]
    fn resampling_to_the_current_size_does_nothing() {
        let mut app = app();
        let before = app.doc.active_volume().brick_count();
        let same = app.doc.voxel_size();
        update(&mut app, Message::Resample(same));
        assert_eq!(app.doc.active_volume().brick_count(), before);
        assert!(app.status.is_empty(), "nothing happened, so nothing should be reported");
    }

    // --- the N-body export -------------------------------------------------

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!("hani-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// An application holding `count` small spheres, laid out along X so they
    /// do not touch.
    ///
    /// A coarse lattice and a small radius on purpose: this is about how many
    /// bodies reach a file, and twelve default spheres at 0.25 mm would spend
    /// the whole test welding.
    fn app_with_bodies(count: usize) -> Hani {
        let mut app = app();
        let mut first = Volume::new(1.0);
        first.seed_sphere(Vec3::ZERO, 8.0);
        let mut doc = Document::from_volume(first);
        for index in 1..count {
            let mut volume = Volume::new(1.0);
            volume.seed_sphere(Vec3::new(index as f32 * 48.0, 0.0, 0.0), 8.0);
            doc.add_body(format!("Body {}", index + 1), volume);
        }
        app.doc = doc;
        app.rebuild_everything();
        app
    }

    /// Turn one row's eye off through the document's own snapshot path, which
    /// is what the panel will do.
    fn hide(app: &mut Hani, at: usize) {
        let id = app.doc.nodes()[at].id;
        let mut meta = app.doc.meta(id).expect("the row is in the document");
        meta.visible = false;
        app.doc.set_meta(&meta);
    }

    /// **Twelve bodies with one hidden export eleven, and the status says so in
    /// those words.**
    ///
    /// Decisions 4 and 10 both rest on this string. The eye is one bit in a
    /// forty-byte node record with no checksum over it, and it is the bit that
    /// decides whether a part reaches the printer -- a flipped bit is a legal
    /// value that loads without any error at all. The brick stream has a
    /// checked distance decode defending it; the node table has nothing. Naming
    /// the count is the whole of that defence, which is why it is asserted
    /// rather than assumed.
    #[test]
    fn a_hidden_body_is_omitted_and_the_status_names_how_many() {
        let directory = scratch("export-hidden");
        let path = directory.join("twelve.obj");
        let mut app = app_with_bodies(12);
        hide(&mut app, 4);

        app.export(ExportFormat::Obj, &path);

        assert!(
            app.status.contains("exported 11 of 12 bodies; 1 hidden"),
            "the omitted count is not in the status: {}",
            app.status
        );
        let written = std::fs::read_to_string(&path).expect("the file should be there");
        let objects: Vec<&str> = written.lines().filter(|line| line.starts_with("o ")).collect();
        assert_eq!(objects.len(), 11, "eleven bodies should have reached the file: {objects:?}");
        assert!(
            !objects.contains(&"o Body 5"),
            "the hidden body reached the file anyway: {objects:?}"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    /// **And a document with nothing hidden still names the count.**
    ///
    /// Split out from the test above because it is the case a conditional
    /// message would get wrong, and a conditional message is the obvious
    /// "improvement" somebody makes later: if the line only appears when
    /// something was hidden, silence means either "nothing was hidden" or "the
    /// count was never worked out", and those are the two readings the count
    /// exists to keep apart.
    /// **The export detail reaches the file**, and a full export is still
    /// exactly what it was.
    ///
    /// Two halves, because the setting has a default that must change
    /// nothing: at Full the file and the status line are what they were
    /// before the setting existed, and at anything else the file is smaller,
    /// still passes the verdict, and the status line says what was done to it
    /// in millimetres.
    #[test]
    fn the_export_detail_decimates_the_file_and_full_leaves_it_alone() {
        let directory = scratch("export-detail");
        let full = directory.join("full.stl");
        let draft = directory.join("draft.stl");
        let mut app = app();

        assert_eq!(
            app.export_detail,
            ExportDetail::Full,
            "a fresh application should not decimate"
        );
        app.export(ExportFormat::Stl, &full);
        assert!(app.status.contains("exported 1 of 1 bodies"), "{}", app.status);
        assert!(
            !app.status.contains("within"),
            "a full export's status line mentions a tolerance: {}",
            app.status
        );

        update(&mut app, Message::ExportDetailChanged(ExportDetail::Draft));
        assert_eq!(app.export_detail, ExportDetail::Draft, "the setting did not arrive");
        app.export(ExportFormat::Stl, &draft);
        assert!(app.status.contains("exported 1 of 1 bodies"), "{}", app.status);
        assert!(
            app.status.contains("triangles, within") && app.status.contains("mm of the sculpt"),
            "a decimated export's status line does not say what was done: {}",
            app.status
        );

        let full_bytes = std::fs::metadata(&full).expect("the full export was written").len();
        let draft_bytes = std::fs::metadata(&draft).expect("the draft export was written").len();
        assert!(
            draft_bytes * 4 < full_bytes,
            "the draft is {draft_bytes} bytes against {full_bytes}: it was not decimated"
        );
        // A binary STL is 84 bytes and then 50 a triangle, so the size is the
        // triangle count and a decimated file still has to be a whole number
        // of them.
        assert_eq!((draft_bytes - 84) % 50, 0, "the draft is not a well formed binary STL");

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_document_with_nothing_hidden_still_names_the_count() {
        let directory = scratch("export-none-hidden");
        let path = directory.join("one.stl");
        let mut app = app();

        app.export(ExportFormat::Stl, &path);

        assert!(
            app.status.contains("exported 1 of 1 bodies; 0 hidden"),
            "the count is missing from a plain single-body export: {}",
            app.status
        );
        std::fs::remove_dir_all(&directory).ok();
    }

    /// **An export while soloed writes every body the EYE is showing, including
    /// the ones solo is hiding, and names only the eye-hidden count.**
    ///
    /// This is the line where a view mode would be at its most dangerous. Solo
    /// is a way of looking at a document; export is what reaches a printer. If
    /// the two ever share a visibility answer, a user who soloed one part to
    /// inspect it and then pressed Export gets a file with eleven parts missing
    /// and a status line that agrees with them -- and the failure is discovered
    /// after the print. `export` reads `saved_visibility`, and `saved_visibility`
    /// takes no solo parameter at all, so this is a test of a call that has no
    /// other form.
    #[test]
    fn an_export_while_soloed_still_writes_every_body_the_eye_shows() {
        let directory = scratch("export-soloed");
        let path = directory.join("twelve.obj");
        let mut app = app_with_bodies(12);
        hide(&mut app, 4);

        // Solo something else entirely, so eleven of the twelve rows are not
        // drawn and only ONE of them is hidden by an eye.
        let soloed = app.doc.nodes()[7].id;
        drop(app.update(Message::SoloEntered(soloed)));
        assert_eq!(app.solo, Some(soloed), "the fixture never entered solo");
        let mut drawn = Vec::new();
        app.doc.display_visibility(app.solo, &mut drawn);
        assert_eq!(
            drawn.iter().filter(|shown| **shown).count(),
            1,
            "the fixture is not actually hiding anything on screen"
        );

        app.export(ExportFormat::Obj, &path);

        assert!(
            app.status.contains("exported 11 of 12 bodies; 1 hidden"),
            "solo reached the export, or the count now includes it: {}",
            app.status
        );
        let written = std::fs::read_to_string(&path).expect("the file should be there");
        let objects: Vec<&str> = written.lines().filter(|line| line.starts_with("o ")).collect();
        assert_eq!(objects.len(), 11, "solo dropped ten parts out of the print: {objects:?}");
        assert!(
            !objects.contains(&"o Body 5"),
            "the eye-hidden body reached the file anyway: {objects:?}"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A single body's STL is byte for byte what the single-mesh writer
    /// produces, end to end through the application.
    ///
    /// **What this proves on its own is narrower than its name suggests**, and
    /// worth stating so nobody reads it as the byte pin it is not. Both sides
    /// end in `stl::write_all`, so a change to the writer moves them together
    /// and this test cannot see it. What it does see is everything the
    /// application wraps around the writer -- `export_bodies`, the per-body
    /// weld, `document_verdict`, the file creation -- adding nothing to a
    /// one-body file. The bytes themselves are pinned in `hani-core` by
    /// `export::stl::tests::a_known_mesh_writes_the_bytes_committed_in_the_golden`
    /// against a committed fixture. The two together are what pins the file a
    /// user receives; neither one does it alone.
    ///
    /// The shipped, verified property: an STL out of this build has been opened
    /// in a slicer and printed. STL carries no object names, so it is the one
    /// format where the N-body path can be identical rather than merely
    /// equivalent -- OBJ and 3MF now carry the body's name where they used to
    /// carry the literal `HaniSculpt`, which is the only byte that moves.
    #[test]
    fn a_single_body_export_is_byte_identical_to_the_single_mesh_writer() {
        let directory = scratch("export-identical");
        let path = directory.join("one.stl");
        let mut app = app();

        app.export(ExportFormat::Stl, &path);
        let written = std::fs::read(&path).expect("the file should be there");

        let (mesh, report) = app.doc.active_volume().export_mesh();
        assert!(report.is_printable());
        let mut expected = Vec::new();
        hani_core::export::stl::write(&mesh, &mut expected).unwrap();
        assert_eq!(written, expected, "the N-body path changed a one-body STL");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// The user's own filament slots reach the exported package.
    ///
    /// **This is the whole of what increment 1 is for.** The `paint_color`
    /// writer and its `Filaments` shipped complete and unreferenced: nothing
    /// outside `export/threemf.rs` mentioned either, so every 3MF a user ever
    /// received declared the four built-in colours regardless of what was in
    /// their machine. The assertion is a substring rather than a parsed zip
    /// because the writer stores its entries uncompressed, so the settings part
    /// is literally in the file -- and reaching for a zip reader to check one
    /// string would be the more fragile of the two.
    #[test]
    fn the_users_own_filaments_reach_the_exported_package() {
        let directory = scratch("export-palette");
        let path = directory.join("painted.3mf");
        let mut app = app();
        app.palette.slots[0].colour = "#123456".to_string();
        app.palette.slots[1].material = "PETG".to_string();

        app.export(ExportFormat::ThreeMf, &path);
        let written =
            String::from_utf8_lossy(&std::fs::read(&path).expect("the file should be there"))
                .into_owned();

        assert!(
            written.contains("\"#123456\""),
            "the export declared the built-in colours instead of the user's"
        );
        assert!(written.contains("\"PETG\""), "the material did not travel");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A user who never touches the palette gets exactly the file they got
    /// before, byte for byte.
    ///
    /// The default palette is DERIVED from `Filaments::default()` rather than
    /// restating it, and this is what holds that true: if the two ever drift,
    /// every existing export silently changes and this fails.
    #[test]
    fn an_untouched_palette_exports_the_same_bytes_as_before() {
        let directory = scratch("export-palette-default");
        let path = directory.join("plain.3mf");
        let mut app = app();

        app.export(ExportFormat::ThreeMf, &path);
        let written =
            String::from_utf8_lossy(&std::fs::read(&path).expect("the file should be there"))
                .into_owned();

        // The default palette must still be the writer's own defaults. Asserted
        // on the settings part rather than on the whole file's bytes, because
        // the placement transform is read from the developer's own OrcaSlicer
        // presets -- a byte comparison here would pass or fail depending on
        // which printer the machine running the suite happens to have selected.
        for colour in hani_core::export::threemf::Filaments::default().colours {
            assert!(
                written.contains(&format!("\"{colour}\"")),
                "the default palette and the writer's own defaults have drifted apart: \
                 {colour} is missing"
            );
        }

        std::fs::remove_dir_all(&directory).ok();
    }

    /// The exported model sits ON the bed, not half under it.
    ///
    /// A 3MF opened as a project is placed exactly where the file says, and
    /// with no transform that was the plate's front-left corner with half the
    /// sculpt below z = 0 -- reported from OrcaSlicer against a real export on
    /// 2026-09-02. The centring half needs the user's plate size and is absent
    /// on a machine with no OrcaSlicer; the drop onto the bed is unconditional
    /// and is what this pins.
    #[test]
    fn an_exported_model_sits_on_the_bed_rather_than_half_under_it() {
        let directory = scratch("export-on-bed");
        let path = directory.join("placed.3mf");
        let mut app = app();

        app.export(ExportFormat::ThreeMf, &path);
        let written =
            String::from_utf8_lossy(&std::fs::read(&path).expect("the file should be there"))
                .into_owned();

        let item = written
            .lines()
            .find(|line| line.contains("<item "))
            .expect("the build has to reference the object")
            .to_string();
        let transform = item
            .split_once("transform=\"")
            .map(|(_, rest)| rest.split('"').next().unwrap_or_default().to_string())
            .expect("a default sphere straddles z = 0, so it must carry a transform");
        let numbers: Vec<f32> =
            transform.split_whitespace().filter_map(|word| word.parse().ok()).collect();
        assert_eq!(numbers.len(), 12, "a 3MF item transform is 12 numbers: {transform}");

        // The lowest point of the sphere, in print space, plus the lift the
        // transform applies, has to land on the bed.
        let lifted = numbers[11];
        let mut lowest = f32::INFINITY;
        for line in written.lines().filter(|line| line.contains("<vertex ")) {
            if let Some((_, rest)) = line.split_once("z=\"")
                && let Ok(z) = rest.split('"').next().unwrap_or_default().parse::<f32>()
            {
                lowest = lowest.min(z);
            }
        }
        assert!(lowest.is_finite(), "no vertices in the exported file");
        assert!(
            (lowest + lifted).abs() < 1.0e-3,
            "the model's lowest point lands at {} instead of on the bed",
            lowest + lifted
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    /// Several bodies in one STL are named as fused, because the format has
    /// nowhere to put an object boundary and a slicer will load them as one
    /// part. Better said here than discovered after slicing.
    #[test]
    fn an_stl_of_several_bodies_says_they_arrive_as_one_part() {
        let directory = scratch("export-stl-fused");
        let path = directory.join("three.stl");
        let mut app = app_with_bodies(3);

        app.export(ExportFormat::Stl, &path);
        assert!(app.status.contains("exported 3 of 3 bodies"), "{}", app.status);
        assert!(
            app.status.contains("STL fuses them into one part"),
            "an STL of three bodies did not say what it did with them: {}",
            app.status
        );
        std::fs::remove_dir_all(&directory).ok();
    }

    /// **A body that would not print refuses the whole export, and refuses it
    /// before the file is opened.**
    ///
    /// `File::create` truncates, so a verdict taken after it destroys the file
    /// the user was about to print -- and the natural per-body refactor is
    /// exactly the one that moves the check down past the open. The previous
    /// export was left in place here, unread and unchanged.
    #[test]
    fn an_unprintable_body_refuses_before_the_file_is_opened() {
        let directory = scratch("export-refusal");
        let path = directory.join("keepme.obj");
        std::fs::write(&path, b"the previous export, which must survive").unwrap();

        let mut app = app_with_bodies(2);
        // An empty body prints nothing, and the sum over the document would
        // still read as watertight -- see `document_verdict`.
        let empty = app.doc.nodes()[1].id;
        *app.doc.volume_mut(empty).expect("a body") = Volume::new(1.0);

        app.export(ExportFormat::Obj, &path);

        assert!(app.status.starts_with("not exported"), "reported: {}", app.status);
        assert!(app.status.contains("Body 2"), "the refusal must name the body: {}", app.status);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"the previous export, which must survive",
            "the refusal opened and truncated the file before deciding"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    /// **The detail guard measures the whole document, not the active body.**
    ///
    /// It never did: `too_fine_for_the_pool` reads `doc_stats`, which is a sum
    /// over every body, and the numbers below are what makes the difference
    /// visible. Two bodies of 0.8 GiB each is 1.6 GiB, and one halving of the
    /// voxel quadruples a shell -- 6.4 GiB, past the ceiling. Either body ALONE
    /// would come to 3.2 GiB and be admitted. A guard that asked the active
    /// body would step straight past the ceiling with the second body's bricks
    /// unaccounted for, and the first thing to report it would be the machine
    /// swapping.
    #[test]
    fn a_two_body_document_over_the_ceiling_refuses_the_finer_step() {
        let mut app = app_with_bodies(2);
        assert_eq!(app.doc.body_count(), 2);

        // `doc_stats` really is the sum, measured rather than assumed.
        let per_body: usize =
            app.doc.bodies().map(|(_, volume)| volume.stats().resident_bytes).sum();
        assert_eq!(app.doc_stats.resident_bytes, per_body);
        assert!(
            app.doc_stats.resident_bytes > app.doc.active_volume().stats().resident_bytes,
            "the fixture's two bodies must not cost the same as one"
        );

        let each = 0.8 * 1024.0 * 1024.0 * 1024.0;
        assert!(each * 4.0 < MAX_VOLUME_BYTES, "either body alone has to be admitted");
        assert!(each * 2.0 * 4.0 > MAX_VOLUME_BYTES, "the pair has to be refused");
        app.doc_stats.resident_bytes = (each * 2.0) as usize;

        let before = app.doc.voxel_size();
        update(&mut app, Message::Resample(before / 2.0));

        assert!(app.doc.voxel_size() > before / 2.0, "the halving should not have been allowed");
        assert!(
            app.status.contains("finest the mesh pool holds"),
            "the cap should be reported: {}",
            app.status
        );
    }
}

/// The mask TOOL: the mode, the gesture, and what one mask stroke costs.
///
/// Its own module because every test here is about the tool rather than about
/// the brush -- what a press means, which map it writes, and what the release
/// pushes onto history.
#[cfg(test)]
mod mask_tool_tests {
    use super::*;
    use crate::tablet::PenState;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    fn centre_of_viewport() -> Vector {
        Vector::new(SIZE.x / 2.0, SIZE.y / 2.0)
    }

    /// Pointer events routed through `update` rather than straight into
    /// `on_pointer`.
    ///
    /// The difference matters here and nowhere else in this file: the standing
    /// mask card is rebuilt in `publish_visibility`, which `update` runs after
    /// every message and which `on_pointer` never reaches. A test that poked
    /// the pointer handler directly would paint a mask and then find no card,
    /// which is a fact about the test harness rather than about the
    /// application.
    fn press(app: &mut Hani, at: Vector) {
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: at,
                size: SIZE,
            }),
        );
    }

    fn drag_to(app: &mut Hani, at: Vector) {
        update(app, Message::Pointer(PointerEvent::Moved { position: at, size: SIZE }));
    }

    fn release(app: &mut Hani) {
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// A short mask drag across the middle of the model.
    fn mask_drag(app: &mut Hani) {
        let centre = centre_of_viewport();
        press(app, centre);
        drag_to(app, Vector::new(centre.x + 20.0, centre.y));
        drag_to(app, Vector::new(centre.x + 40.0, centre.y));
        release(app);
    }

    /// Every field value of the active body, in a fixed order, for a bit-for-bit
    /// comparison across a gesture.
    fn field_of(app: &Hani) -> Vec<(hani_core::BrickCoord, Vec<f32>)> {
        let volume = app.doc.active_volume();
        let mut coords: Vec<_> = volume.brick_coords().collect();
        coords.sort_by_key(|coord| (coord.0.z, coord.0.y, coord.0.x));
        coords
            .into_iter()
            .map(|coord| {
                let origin = coord.origin();
                let dim = hani_core::BRICK_DIM as i32;
                let values = (0..dim)
                    .flat_map(|z| {
                        (0..dim).flat_map(move |y| (0..dim).map(move |x| glam::IVec3::new(x, y, z)))
                    })
                    .map(|local| volume.sample_voxel(origin + local))
                    .collect();
                (coord, values)
            })
            .collect()
    }

    /// `m` enters and leaves, and the brush selection underneath is untouched.
    ///
    /// The second half is the one worth pinning. Mask is a TOOL and not an
    /// eighth brush, so entering it must not disturb which brush a user will be
    /// back on when they leave -- and the digits, `s`, `[` and `]` all keep
    /// working on the mask because it borrows the brush's radius and falloff.
    #[test]
    fn m_enters_and_leaves_mask_mode_and_leaves_the_brush_alone() {
        let mut app = app();
        update(&mut app, Message::BrushKindChanged(BrushKind::Clay));
        let radius = app.brush.radius;

        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert_eq!(app.tool, Tool::Mask, "m did not enter mask mode");
        assert_eq!(app.brush.kind, BrushKind::Clay, "entering mask mode changed the brush");

        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert_eq!(app.tool, Tool::Sculpt, "pressing the live tool did not go back to sculpting");
        assert_eq!(app.brush.kind, BrushKind::Clay, "leaving mask mode changed the brush");
        assert_eq!(app.brush.radius, radius, "the mask tool kept the brush's radius");
    }

    /// `m` really is the key, decoded the same way every other shortcut is.
    #[test]
    fn the_m_key_decodes_to_the_mask_tool() {
        assert!(
            matches!(
                crate::viewport::shortcut("m", false, false, false),
                Some(Message::ToolChanged(Tool::Mask))
            ),
            "m does not reach the mask tool"
        );
        // Chorded it belongs to the toolkit, like everything else under a
        // modifier that this application has not claimed.
        assert!(crate::viewport::shortcut("m", true, false, false).is_none());
        assert!(crate::viewport::shortcut("m", false, false, true).is_none());
    }

    /// `w` reaches the transform gizmo, and only bare.
    ///
    /// ZBrush's, Blender's and Maya's key for Move, taken rather than invented.
    /// Chorded it belongs to the toolkit: `ctrl+w` closes a window nearly
    /// everywhere and this application has claimed neither form.
    #[test]
    fn the_w_key_decodes_to_the_transform_gizmo() {
        assert!(
            matches!(
                crate::viewport::shortcut("w", false, false, false),
                Some(Message::ToolChanged(Tool::Transform))
            ),
            "w does not reach the transform gizmo"
        );
        assert!(crate::viewport::shortcut("w", true, false, false).is_none());
        assert!(crate::viewport::shortcut("w", false, false, true).is_none());
    }

    /// `h` starts a peek, and only bare.
    ///
    /// ZBrush's own mask-visibility key, taken rather than invented. Chorded it
    /// belongs to the toolkit: `ctrl+h` is a browser and toolkit shortcut in
    /// several places and this application has claimed neither form.
    #[test]
    fn the_h_key_decodes_to_a_mask_peek() {
        assert!(
            matches!(
                crate::viewport::shortcut("h", false, false, false),
                Some(Message::MaskPeekStarted)
            ),
            "h does not start a peek"
        );
        assert!(crate::viewport::shortcut("h", true, false, false).is_none());
        assert!(crate::viewport::shortcut("h", false, false, true).is_none());
    }

    /// **A mask drag changes the mask and not one field value.**
    ///
    /// The whole of what the tool promises. A field that moved here is a mask
    /// tool that has quietly become a sculpt brush, which a user would find in
    /// an exported print rather than on screen.
    #[test]
    fn a_mask_drag_changes_the_mask_and_not_one_field_value() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        let before = field_of(&app);

        mask_drag(&mut app);

        assert!(app.doc.active_volume().mask_fill() > 0.0, "the drag painted no mask at all");
        assert_eq!(field_of(&app), before, "a mask drag moved the field");
    }

    /// **Ctrl lowers the mask with every one of the seven brushes selected.**
    ///
    /// This is the `is_directional` trap, and it is the reason mask mode
    /// computes its own direction. `stroke_direction` gates on
    /// `brush.kind.is_directional()`, which answers false for Smooth, Flatten
    /// and Move -- so a mask painter that reused it would MASK on ctrl+drag for
    /// three of the seven, with nothing on screen to say which three.
    #[test]
    fn ctrl_lowers_the_mask_with_every_brush_selected() {
        for kind in BrushKind::ALL {
            let mut app = app();
            update(&mut app, Message::BrushKindChanged(kind));
            update(&mut app, Message::ToolChanged(Tool::Mask));

            mask_drag(&mut app);
            let raised = app.doc.active_volume().mask_fill();
            assert!(raised > 0.0, "{kind}: the plain drag painted nothing");

            app.control = true;
            for _ in 0..6 {
                mask_drag(&mut app);
            }
            assert!(
                app.doc.active_volume().mask_fill() < raised,
                "{kind}: ctrl+drag did not lower the mask"
            );

            // ...and alt is its synonym, exactly as it is for the brush.
            app.control = false;
            app.alt = true;
            assert_eq!(app.mask_op(), MaskOp::Lower, "{kind}: alt is not ctrl's synonym");
        }
    }

    /// The eraser end of the stylus unmasks, and combines with the modifier the
    /// way it does everywhere else.
    #[test]
    fn the_stylus_eraser_lowers_the_mask() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert_eq!(app.mask_op(), MaskOp::Raise);

        app.tablet.simulate(PenState {
            in_proximity: true,
            pressure: 1.0,
            eraser: true,
            tilt: glam::Vec2::ZERO,
        });
        assert_eq!(app.mask_op(), MaskOp::Lower, "the eraser end did not unmask");

        // The two combine rather than override: holding the modifier while
        // using the eraser gives back the additive gesture.
        app.control = true;
        assert_eq!(app.mask_op(), MaskOp::Raise);

        // And shift outranks both, because blur is a different operation rather
        // than a direction.
        app.shift = true;
        assert_eq!(app.mask_op(), MaskOp::Blur);
    }

    /// **A mask-only stroke sets `unsaved` and pushes exactly one history
    /// entry**, and undoing it puts the mask back and marks bricks dirty.
    ///
    /// It did neither before this increment: `end_stroke` recorded field bricks
    /// only, so a mask gesture produced no entry, never set `unsaved`, and did
    /// not even raise the discard prompt on the way out.
    #[test]
    fn a_mask_stroke_is_one_undoable_entry_that_marks_the_document_unsaved() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        let entries = app.history_stats.undo_entries;

        mask_drag(&mut app);

        let painted = app.doc.active_volume().mask_fill();
        assert!(painted > 0.0, "the fixture painted nothing");
        assert!(app.unsaved, "a mask stroke did not mark the document unsaved");
        assert_eq!(
            app.history_stats.undo_entries,
            entries + 1,
            "one mask gesture became {} entries",
            app.history_stats.undo_entries - entries
        );

        app.perf.dirty_bricks = 0;
        app.undo();
        assert_eq!(app.doc.active_volume().mask_fill(), 0.0, "undo did not take the mask off");
        assert!(app.perf.dirty_bricks > 0, "undoing a mask stroke remeshed nothing");

        app.redo();
        assert!(
            (app.doc.active_volume().mask_fill() - painted).abs() < 1.0e-9,
            "redo did not put the mask back"
        );
    }

    /// Escape leaves mask mode ON, and clears the cut.
    ///
    /// A cut is a pending destructive thing and Escape calls it off. A mask is
    /// expensive user work, and Escape is also the key that closes every menu,
    /// so a user dismissing a dropdown must not silently lose the mode they are
    /// painting in.
    #[test]
    fn escape_clears_the_cut_and_leaves_mask_mode_on() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        update(&mut app, Message::MenuClosed);
        assert_eq!(app.tool, Tool::Sculpt, "escape did not disarm the cut");

        update(&mut app, Message::ToolChanged(Tool::Mask));
        update(&mut app, Message::MenuClosed);
        assert_eq!(app.tool, Tool::Mask, "escape threw away the mask mode");
    }

    /// An in-progress resize outranks BOTH modes, which is a change of ordering
    /// from the cut alone.
    ///
    /// A destructive one-shot may reasonably outrank a resize; an ongoing paint
    /// may not, and once the two collapsed into one match on the tool the merged
    /// arm had to take one of the two precedences. It takes the mask's.
    #[test]
    fn a_sizing_drag_outranks_both_modes() {
        for tool in [Tool::Cut, Tool::Mask] {
            let mut app = app();
            app.cursor = Some(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0));
            update(&mut app, Message::ToolChanged(tool));
            update(&mut app, Message::SizingStarted(SizingTarget::Radius));
            assert!(app.sizing.is_some(), "the fixture did not start a resize");

            press(&mut app, centre_of_viewport());
            assert_eq!(
                app.drag.map(|drag| drag.kind),
                Some(DragKind::Sizing),
                "{tool:?} stole a press that belonged to a resize"
            );
        }
    }

    /// The mask keeps its own strength, and the brush gets its own back.
    ///
    /// Draw's default is 0.15, which barely deposits; a user painting a mask
    /// drags the slider up to get a usable one. Sharing the number would make
    /// the very next Draw stroke after leaving mask mode a gouge at full
    /// strength on a scan they were carefully repairing.
    #[test]
    fn the_mask_remembers_its_own_strength_and_the_brush_keeps_its() {
        let mut app = app();
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        assert!((app.brush.strength - 0.15).abs() < 1.0e-6);

        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert!(app.brush.strength > 0.9, "the mask arrived at {}", app.brush.strength);

        update(&mut app, Message::BrushStrengthChanged(0.6));
        // Choosing a different brush mid-mask must not disturb it: both sides of
        // the swap resolve to the mask's own slot.
        update(&mut app, Message::BrushKindChanged(BrushKind::Clay));
        assert!((app.brush.strength - 0.6).abs() < 1.0e-6, "the mask forgot its setting");

        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert_eq!(app.tool, Tool::Sculpt);
        assert!(
            (app.brush.strength - 0.15).abs() < 1.0e-6,
            "clay came back at the mask's strength"
        );

        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert!((app.brush.strength - 0.6).abs() < 1.0e-6, "the mask did not get its own back");
    }

    /// The standing card is up whenever anything is masked, whatever tool is
    /// live, and it names the body.
    ///
    /// This is the mechanism that makes "a mask is active and nothing on screen
    /// says so" unreachable, so the assertion is about it being UNCONDITIONAL
    /// rather than about its wording.
    #[test]
    fn the_mask_card_stands_whatever_tool_is_live() {
        let mut app = app();
        assert!(app.mask_card.is_none(), "an unmasked document showed a mask card");

        update(&mut app, Message::ToolChanged(Tool::Mask));
        mask_drag(&mut app);
        let card = app.mask_card.clone().expect("a painted mask showed no card");
        let name = app.doc.node(app.doc.active()).expect("an active body").name.clone();
        assert!(
            card.headline.contains(&name),
            "the card does not name the body: {}",
            card.headline
        );
        assert!(card.headline.contains('%'), "the card carries no percentage: {}", card.headline);

        // Back to sculpting, and the card stays: the whole point is that it is
        // not scoped to the tool that made the mask.
        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert_eq!(app.tool, Tool::Sculpt);
        assert!(app.mask_card.is_some(), "leaving mask mode took the card with it");

        // And it goes away when the mask does.
        update(&mut app, Message::Undo);
        assert!(app.mask_card.is_none(), "undoing the mask left its card behind");
    }

    /// **Switching the tint off leaves the card up, unchanged, and marks no
    /// brick dirty.** This is the whole of what makes decision 13 safe.
    ///
    /// A `show mask` toggle ships where this plan argued for a held-only peek
    /// key, on the reasoning that a tint which can be switched off will be left
    /// off. What closes the hole is that the toggle governs the TINT ONLY: the
    /// card is built from the document and consults neither the toggle nor the
    /// strength, so "a mask is active and nothing on screen says so" stays
    /// unreachable. If this test ever has to be relaxed, the toggle is the
    /// thing to delete rather than the assertion.
    ///
    /// The pixel half of the claim -- that a body with the tint off is
    /// pixel-identical to an unmasked one -- is in `hani-gpu`'s
    /// `offscreen.rs`, which is the only place a pixel exists.
    #[test]
    fn switching_the_tint_off_leaves_the_card_up_and_every_brick_alone() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        mask_drag(&mut app);

        let card = app.mask_card.clone().expect("a painted mask showed no card");
        let fill = app.doc.active_volume().mask_fill();
        let revision = app.doc.active_volume().mask().revision();
        assert_eq!(app.doc.active_volume().dirty_count(), 0, "the fixture left a remesh pending");
        assert_eq!(app.shared.mask_view().tint, DEFAULT_MASK_TINT, "the tint did not start on");

        update(&mut app, Message::ShowMaskToggled);
        assert_eq!(app.shared.mask_view().tint, 0.0, "the toggle did not reach the renderer");
        assert_eq!(app.mask_card, Some(card.clone()), "switching the tint off changed the card");
        assert_eq!(app.doc.active_volume().mask_fill(), fill, "the toggle changed protection");
        assert_eq!(app.doc.active_volume().mask().revision(), revision);
        assert_eq!(
            app.doc.active_volume().dirty_count(),
            0,
            "the toggle marked bricks for remesh, which it has no reason to: the tint is a uniform"
        );

        update(&mut app, Message::ShowMaskToggled);
        assert_eq!(app.shared.mask_view().tint, DEFAULT_MASK_TINT, "the tint did not come back");
        assert_eq!(app.mask_card, Some(card), "switching the tint back on changed the card");
    }

    /// The held peek key shows the mask whatever the toggle says, and puts it
    /// back exactly as it was.
    ///
    /// It is kept alongside the toggle rather than replaced by it because it is
    /// the way back for a user who switched the tint off and forgot -- so the
    /// case worth pinning is the one where the toggle is OFF.
    #[test]
    fn the_held_peek_key_shows_the_mask_whatever_the_toggle_says() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        mask_drag(&mut app);
        update(&mut app, Message::ShowMaskToggled);
        update(&mut app, Message::MaskTintChanged(0.6));
        assert_eq!(app.shared.mask_view().tint, 0.0, "the fixture did not switch the tint off");

        update(&mut app, Message::MaskPeekStarted);
        assert_eq!(app.shared.mask_view().tint, 0.6, "the peek key showed nothing");
        // It repeats while it is held, which must be idempotent rather than a
        // strobe -- which is why the peek is two messages and not a toggle.
        update(&mut app, Message::MaskPeekStarted);
        assert_eq!(app.shared.mask_view().tint, 0.6);

        update(&mut app, Message::MaskPeekEnded);
        assert_eq!(app.shared.mask_view().tint, 0.0, "the peek key latched");
        assert_eq!(app.doc.active_volume().dirty_count(), 0, "a peek marked bricks for remesh");
    }

    /// The tint strength is a view strength in 0..1, and nothing it does
    /// reaches protection.
    #[test]
    fn the_tint_strength_is_clamped_and_changes_no_protection() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        mask_drag(&mut app);
        let fill = app.doc.active_volume().mask_fill();

        update(&mut app, Message::MaskTintChanged(4.0));
        assert_eq!(app.shared.mask_view().tint, 1.0);
        update(&mut app, Message::MaskTintChanged(-2.0));
        assert_eq!(app.shared.mask_view().tint, 0.0);
        update(&mut app, Message::MaskTintChanged(0.25));
        assert_eq!(app.shared.mask_view().tint, 0.25);

        assert_eq!(
            app.doc.active_volume().mask_fill(),
            fill,
            "the tint slider changed how much of the body is protected, which is the confusion \
             3D-Coat has to warn about in its own documentation"
        );
    }

    /// The polarity handed to the shader is read out of the ACTIVE body every
    /// time, not remembered beside it.
    ///
    /// Remembering it is how the two come to disagree, and what a disagreement
    /// looks like is protected material drawn as free -- the mask at its most
    /// misleading. Nothing in the application can set the polarity yet, so this
    /// sets it the way the increment that ships Invert will.
    #[test]
    fn the_published_polarity_is_the_active_bodys_own() {
        let mut app = app();
        assert!(!app.shared.mask_view().inverted);

        app.doc.active_volume_mut().mask_mut().set_inverted(true);
        update(&mut app, Message::Frame);
        assert!(app.shared.mask_view().inverted, "the flip never reached the renderer");

        app.doc.active_volume_mut().mask_mut().set_inverted(false);
        update(&mut app, Message::Frame);
        assert!(!app.shared.mask_view().inverted);
    }

    /// The card's percentage is CACHED and only recomputed when the mask moves.
    ///
    /// `view()` runs at display rate off `window::frames()`, so a frame that
    /// changed nothing must not pay for a pass over the mask.
    ///
    /// **Asserted with a sentinel, because comparing the card against itself
    /// cannot fail.** A rebuild writes the same words as the card it replaces,
    /// so `assert_eq!(card, card_before)` passes just as happily with the
    /// revision gate deleted and `mask_fill` running on every one of the sixty
    /// frames a second -- which is the entire cost the cache exists to avoid. A
    /// word only the test could have written is the one thing a recompute
    /// destroys.
    #[test]
    fn a_frame_that_changed_nothing_does_not_rebuild_the_mask_card() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        mask_drag(&mut app);
        assert!(app.mask_card.is_some(), "a painted mask showed no card");

        let revision = app.doc.active_volume().mask().revision();
        app.mask_card.as_mut().expect("a painted mask showed no card").headline =
            "SENTINEL".to_string();
        for _ in 0..8 {
            update(&mut app, Message::Frame);
        }
        assert_eq!(
            app.doc.active_volume().mask().revision(),
            revision,
            "an idle frame moved the mask revision, so the cache can never hit"
        );
        assert_eq!(
            app.mask_card.as_ref().map(|card| card.headline.as_str()),
            Some("SENTINEL"),
            "eight idle frames rebuilt the card, so every frame pays for mask_fill"
        );
        assert_eq!(app.mask_card.as_ref().map(|card| card.revision), Some(revision));

        // And the gate is not so wide that a real change slips through it.
        mask_drag(&mut app);
        assert_ne!(
            app.mask_card.as_ref().map(|card| card.headline.as_str()),
            Some("SENTINEL"),
            "painting more mask left the card saying what it said before"
        );
    }

    /// A hidden masked body costs the frame tick nothing either.
    ///
    /// The count of masked bodies off screen is the one line on the card that
    /// goes stale without the mask moving, so it sits outside the revision key
    /// -- and that is exactly how a `format!` gets back onto the frame path.
    /// `publish_visibility` allocates nothing by contract and this runs at the
    /// bottom of it, so the count is compared as a number and only turned into
    /// words when it is a NEW number. Sentinels on both strings, because a
    /// rebuild of either one is the bug.
    #[test]
    fn hiding_a_masked_body_does_not_put_a_format_on_the_frame_path() {
        let mut app = app();
        let first = app.doc.active();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        mask_drag(&mut app);
        // The copy carries the mask, so this is two masked bodies in the same
        // place -- and hiding the original leaves the active one masked, which
        // is the arm where the count sits outside the revision key.
        update(&mut app, Message::BodyDuplicated);
        update(&mut app, Message::BodyVisibilityToggled(first));

        let card = app.mask_card.as_ref().expect("a hidden masked body showed no card");
        assert!(card.headline.ends_with('%'), "no percentage on the card: {}", card.headline);
        assert_eq!(card.off_screen, "+1 masked, hidden", "the hidden body was not counted");

        let card = app.mask_card.as_mut().expect("a hidden masked body showed no card");
        card.headline = "SENTINEL".to_string();
        card.off_screen = "SENTINEL".to_string();
        for _ in 0..8 {
            update(&mut app, Message::Frame);
        }
        let card = app.mask_card.as_ref().expect("eight frames took the card away");
        assert_eq!(card.headline, "SENTINEL", "an idle frame rebuilt the headline");
        assert_eq!(card.off_screen, "SENTINEL", "an idle frame rebuilt the hidden count");

        // A body with no mask of its own, with one hidden and one visible out
        // there: the card is nothing but the counts, both of them are said, and
        // it is cached on both.
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let card = app.mask_card.as_ref().expect("the hidden masked body stopped being named");
        assert_eq!(card.headline, "MASK — 1 masked, elsewhere");
        assert_eq!(card.off_screen, "+1 masked, hidden", "the hidden body stopped being counted");

        app.mask_card.as_mut().expect("a card").headline = "SENTINEL".to_string();
        for _ in 0..8 {
            update(&mut app, Message::Frame);
        }
        assert_eq!(
            app.mask_card.as_ref().map(|card| card.headline.as_str()),
            Some("SENTINEL"),
            "an idle frame rebuilt the whole card for a body that has no mask on it"
        );

        // Showing it again is a real change, and the counts have to move with
        // it -- the hidden line goes and the body joins the visible ones.
        // **The card does NOT go away**, which is increment 24's review
        // finding: it used to, leaving two masked bodies on screen with the
        // active body unmasked and nothing anywhere saying protection existed.
        update(&mut app, Message::BodyVisibilityToggled(first));
        let card = app.mask_card.as_ref().expect("two masked bodies on screen showed no card");
        assert_eq!(card.headline, "MASK — 2 masked, elsewhere");
        assert!(card.off_screen.is_empty(), "nothing is hidden now: {}", card.off_screen);
        assert!(
            !card.names_the_active_body(),
            "the card offered to clear a mask the active body does not have"
        );
    }

    /// A sculpt press onto fully protected material says so.
    ///
    /// Without this the mask's most confusing behaviour -- a stroke that does
    /// nothing because of protection painted minutes ago -- reads as a broken
    /// brush, and there is nothing under the cursor for the user to connect it
    /// to.
    #[test]
    fn a_sculpt_press_onto_a_protected_surface_says_the_mask_blocked_it() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        // Wide and repeated, so the surface under the cursor really does reach
        // full protection rather than the feathered rim's fraction of it.
        update(&mut app, Message::BrushRadiusChanged(12.0));
        for _ in 0..4 {
            mask_drag(&mut app);
        }
        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert_eq!(app.tool, Tool::Sculpt, "the fixture did not go back to sculpting");

        app.status.clear();
        press(&mut app, centre_of_viewport());
        release(&mut app);
        assert!(
            app.status.contains("mask"),
            "a press onto protected material reported: {}",
            app.status
        );
    }
}

/// The paint brush through the application: the digit, the drag, the eraser,
/// the undo, and what the shader is told.
#[cfg(test)]
mod paint_brush_tests {
    use super::*;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    /// The paint brush in hand. The colour brush's own tests are in
    /// `app/painting.rs`.
    fn app() -> Hani {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(BrushKind::Paint));
        app
    }

    fn centre_of_viewport() -> Vector {
        Vector::new(SIZE.x / 2.0, SIZE.y / 2.0)
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
    }

    fn drag_to(app: &mut Hani, at: Vector) {
        update(app, Message::Pointer(PointerEvent::Moved { position: at, size: SIZE }));
    }

    fn release(app: &mut Hani) {
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// A short drag across the middle of the model.
    fn paint_drag(app: &mut Hani) {
        let centre = centre_of_viewport();
        press(app, centre);
        drag_to(app, Vector::new(centre.x + 20.0, centre.y));
        drag_to(app, Vector::new(centre.x + 40.0, centre.y));
        release(app);
    }

    /// Which change of the active body's paint it is on: what a view toggle
    /// must not move.
    fn painted_bricks(app: &Hani) -> u64 {
        app.doc.active_volume().paint().revision()
    }

    #[test]
    fn a_paint_drag_moves_no_field_value() {
        let mut app = app();
        let before: Vec<f32> = {
            let volume = app.doc.active_volume();
            let mut coords: Vec<_> = volume.brick_coords().collect();
            coords.sort_by_key(|coord| (coord.0.z, coord.0.y, coord.0.x));
            coords
                .iter()
                .flat_map(|coord| {
                    let origin = coord.origin();
                    (0..32).map(move |i| volume.sample_voxel(origin + glam::IVec3::new(i, i, i)))
                })
                .collect()
        };
        paint_drag(&mut app);
        let after: Vec<f32> = {
            let volume = app.doc.active_volume();
            let mut coords: Vec<_> = volume.brick_coords().collect();
            coords.sort_by_key(|coord| (coord.0.z, coord.0.y, coord.0.x));
            coords
                .iter()
                .flat_map(|coord| {
                    let origin = coord.origin();
                    (0..32).map(move |i| volume.sample_voxel(origin + glam::IVec3::new(i, i, i)))
                })
                .collect()
        };
        assert_eq!(before.len(), after.len(), "a paint stroke added or removed field bricks");
        assert!(
            before.iter().zip(&after).all(|(a, b)| a.to_bits() == b.to_bits()),
            "a paint stroke moved the field"
        );
    }

    /// Shift over the paint brush smooths at Smooth's own strength, within
    /// the slider's ceiling. Paint's 1.0 is not a sculpting strength.
    #[test]
    fn shift_over_the_paint_brush_smooths_at_smooths_own_strength() {
        let mut app = app();
        assert_eq!(app.brush.strength, 1.0, "the fixture expects paint's fixed 1.0");
        app.shift = true;
        let effective = app.effective_brush();
        assert_eq!(effective.kind, BrushKind::Smooth);
        assert!(effective.strength <= MAX_STRENGTH, "shift-smooth ran at {}", effective.strength);
        assert_eq!(effective.strength, BrushKind::Smooth.default_strength());
    }

    /// A palette change recolours the viewport through the uniforms every
    /// message; the thumbnails are drawn on brick changes, so they have to be
    /// told.
    #[test]
    fn syncing_the_palette_marks_every_thumbnail_stale() {
        let mut app = app();
        paint_drag(&mut app);
        app.thumbs.settle_now();
        while let Some(body) = app.thumbs.next_stale() {
            app.thumbs.requested(body);
        }
        assert_eq!(app.thumbs.stale_count(), 0, "the fixture should start with nothing stale");

        let reported = vec![crate::printer::Filament {
            vendor: "Different".to_string(),
            material: "PLA".to_string(),
            sub_type: String::new(),
            colour: "#123456".to_string(),
            present: true,
        }];
        update(&mut app, Message::PaletteSynced(Ok(Some(reported))));
        assert!(app.thumbs.stale_count() > 0, "a palette change left the thumbnails as they were");
    }

    /// The toggle reaches the shader and touches nothing else: no brick dirty,
    /// no document change, no slot moved.
    #[test]
    fn the_show_paint_toggle_reaches_the_renderer_and_dirties_nothing() {
        let mut app = app();
        paint_drag(&mut app);
        let bricks = painted_bricks(&app);
        app.unsaved = false;
        let mut dirty = Vec::new();
        app.doc.active_volume_mut().take_dirty(&mut dirty);

        assert!(app.shared.paint_view().shown, "paint should start shown");
        update(&mut app, Message::ShowPaintToggled);
        assert!(!app.shared.paint_view().shown, "the toggle did not reach the shared frame");
        assert!(!app.unsaved, "a view toggle dirtied the document");
        assert_eq!(app.doc.active_volume().dirty_count(), 0, "a view toggle dirtied bricks");
        assert_eq!(painted_bricks(&app), bricks, "a view toggle changed the paint");

        // And the palette the shader sees is the machine's, slot by slot.
        let palette = app.shared.paint_view().palette;
        assert_eq!(palette[1], app.palette.slots[0].swatch().into_linear());
    }

    /// The crease toggle reaches the shader AND the meshes, and still touches
    /// nothing a save or an undo would notice.
    ///
    /// The meshes are the half a uniform-only toggle would get wrong: the
    /// signal is measured at mesh time, so a toggle that only wrote the
    /// uniform would switch the shader on over vertices that carry nothing and
    /// the model would not change at all. That shipped twice in this project's
    /// history for other features, as a control that was green and dead.
    #[test]
    fn the_show_creases_toggle_remeshes_with_the_signal_and_changes_nothing_else() {
        let mut app = app();
        app.unsaved = false;
        let undo_before = app.history.stats();
        assert_eq!(app.shared.cavity_view().strength, 0.0, "creases should start hidden");
        let published_off = app.shared.pending_cavity_for_tests();
        assert!(
            published_off.iter().any(|(vertices, _)| *vertices > 0),
            "the fixture published no geometry at all"
        );
        assert!(
            published_off.iter().all(|(_, cavity)| *cavity == 0),
            "a mesh carried cavity bytes before anything asked for them"
        );

        update(&mut app, Message::ShowCavityToggled);
        let view = app.shared.cavity_view();
        assert_eq!(view.strength, CAVITY_STRENGTH, "the toggle did not reach the shared frame");
        assert_eq!(
            view.gain,
            CAVITY_FEATURE_MM / app.doc.voxel_size() / 4.0,
            "the gain is not the feature size over the voxel size"
        );
        let published_on = app.shared.pending_cavity_for_tests();
        let fresh = &published_on[published_off.len()..];
        let with_geometry = fresh.iter().filter(|(vertices, _)| *vertices > 0).count();
        assert!(with_geometry > 0, "switching creases on remeshed nothing");
        assert!(
            fresh.iter().all(|(vertices, cavity)| vertices == cavity),
            "a mesh published after the toggle does not carry one cavity byte per vertex"
        );

        assert!(!app.unsaved, "a view toggle dirtied the document");
        assert_eq!(app.history.stats(), undo_before, "a view toggle reached the history");
        assert_eq!(app.doc.active_volume().dirty_count(), 0, "the remesh left bricks dirty");

        // And off again stops paying for it.
        update(&mut app, Message::ShowCavityToggled);
        assert_eq!(app.shared.cavity_view().strength, 0.0, "the toggle did not switch back off");
        let published_again = app.shared.pending_cavity_for_tests();
        let fresh = &published_again[published_on.len()..];
        assert!(fresh.iter().any(|(vertices, _)| *vertices > 0), "switching off remeshed nothing");
        assert!(
            fresh.iter().all(|(_, cavity)| *cavity == 0),
            "meshes still carry cavity bytes after the toggle was switched off"
        );
    }
}

/// The whole-mask VERBS: Clear, Invert, Mask all, and the four absolute
/// filters.
///
/// Its own module rather than more of `mask_tool_tests`, because none of these
/// is a gesture in the viewport: they are buttons and a slider, and what they
/// have to answer for is memory and undo rather than what a press means.
#[cfg(test)]
mod mask_verb_tests {
    use super::*;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// An application with a real painted mask on its one body.
    fn masked() -> Hani {
        let mut app = app();
        let centre = Vector::new(SIZE.x / 2.0, SIZE.y / 2.0);
        update(&mut app, Message::ToolChanged(Tool::Mask));
        update(&mut app, Message::BrushRadiusChanged(10.0));
        update(
            &mut app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: centre,
                size: SIZE,
            }),
        );
        for step in 1..=3 {
            update(
                &mut app,
                Message::Pointer(PointerEvent::Moved {
                    position: Vector::new(centre.x + step as f32 * 15.0, centre.y),
                    size: SIZE,
                }),
            );
        }
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
        assert!(app.doc.active_volume().mask_fill() > 0.0, "the fixture painted no mask");
        app
    }

    /// Every stored mask byte of the active body, for a bit-for-bit comparison.
    fn mask_of(app: &Hani) -> Vec<(hani_core::BrickCoord, Vec<u8>)> {
        let mask = app.doc.active_volume().mask();
        let mut coords: Vec<_> = mask.brick_coords().collect();
        coords.sort_by_key(|coord| (coord.0.z, coord.0.y, coord.0.x));
        let dim = hani_core::BRICK_DIM as i32;
        coords
            .into_iter()
            .map(|coord| {
                let origin = coord.origin();
                let bytes = (0..dim)
                    .flat_map(|z| {
                        (0..dim).flat_map(move |y| (0..dim).map(move |x| glam::IVec3::new(x, y, z)))
                    })
                    .map(|local| mask.at(origin + local))
                    .collect();
                (coord, bytes)
            })
            .collect()
    }

    /// Clear takes the mask off, gives its bytes back, and one ctrl+Z puts it
    /// back exactly as it was.
    #[test]
    fn clearing_the_mask_frees_its_bytes_and_one_undo_restores_it_bit_for_bit() {
        let mut app = masked();
        let before = mask_of(&app);
        let entries = app.history.stats().undo_entries;
        let masked_bytes = app.doc_stats.resident_bytes;

        update(&mut app, Message::MaskCleared);
        assert!(app.doc.active_volume().mask().is_free(), "Clear left protection behind");
        assert_eq!(app.history.stats().undo_entries, entries + 1, "Clear was not one entry");
        assert!(app.unsaved, "Clear did not mark the document unsaved");
        assert!(
            app.doc_stats.resident_bytes < masked_bytes,
            "Clear kept the mask's bytes: {} against {masked_bytes}",
            app.doc_stats.resident_bytes
        );
        assert!(app.mask_card.is_none(), "the card outlived the mask it was about");

        update(&mut app, Message::Undo);
        assert_eq!(mask_of(&app), before, "undoing a Clear did not put the mask back");
        assert_eq!(app.doc_stats.resident_bytes, masked_bytes);
        assert!(app.mask_card.is_some(), "the card did not come back with the mask");
    }

    /// **Clear charges the reclaim allowance and not the stroke budget.**
    ///
    /// The map is moved into the entry rather than copied into it, so there is
    /// nothing for the 256 MB stroke budget to hold -- and charging it there
    /// would evict every stroke behind it the first time somebody cleared a
    /// mask off a large body.
    #[test]
    fn clearing_the_mask_costs_the_reclaim_allowance_and_not_the_stroke_budget() {
        let mut app = masked();
        let before = app.history.stats();
        update(&mut app, Message::MaskCleared);
        let after = app.history.stats();
        assert_eq!(after.bytes, before.bytes, "a moved mask was charged to the stroke budget");
        assert!(
            after.reclaim_bytes > before.reclaim_bytes,
            "the reclaim allowance is not counting the mask the entry is holding"
        );
    }

    /// **Invert twice is the mask that went in, and costs nothing either way.**
    ///
    /// Without the polarity bool this is a rewrite of every mask brick, which
    /// on a lightly masked 45,567-brick model is 1.04 GiB from one keystroke.
    #[test]
    fn inverting_twice_is_bit_identical_and_marks_no_bricks() {
        let mut app = masked();
        let before = mask_of(&app);
        let history_before = app.history.stats();
        // Drained rather than read off `perf`, which still holds whatever the
        // fixture's last remesh counted -- an Invert does not call one, so the
        // counter would pass this test with the claim reversed.
        let mut dirty = Vec::new();
        app.doc.take_dirty(&mut dirty);

        update(&mut app, Message::MaskInverted);
        assert!(app.doc.active_volume().mask().inverted());
        assert!(app.shared.mask_view().inverted, "the flip never reached the renderer");
        app.doc.take_dirty(&mut dirty);
        assert!(dirty.is_empty(), "an Invert marked {} bricks for a remesh", dirty.len());

        update(&mut app, Message::MaskInverted);
        assert!(!app.doc.active_volume().mask().inverted());
        assert_eq!(mask_of(&app), before, "two Inverts did not come back to the same mask");

        let after = app.history.stats();
        assert_eq!(
            after.undo_entries,
            history_before.undo_entries + 2,
            "two Inverts were not two entries"
        );
        assert_eq!(after.bytes, history_before.bytes, "an Invert cost the stroke budget bytes");
        assert_eq!(after.reclaim_bytes, history_before.reclaim_bytes);

        // And back out again through undo, which has to be just as cheap: a
        // remesh here is 475 ms at the brick count the pool is sized for.
        app.doc.take_dirty(&mut dirty);
        update(&mut app, Message::Undo);
        assert!(app.doc.active_volume().mask().inverted(), "undo did not flip it back");
        app.doc.take_dirty(&mut dirty);
        assert!(dirty.is_empty(), "undoing an Invert marked {} bricks", dirty.len());
    }

    /// Mask All over an empty map is one bool: no map, no bricks, no bytes.
    #[test]
    fn masking_all_over_an_empty_map_allocates_nothing() {
        let mut app = app();
        // `doc.totals()` and not `doc_stats`, which is written by `remesh_dirty`
        // and is still zero on an application nothing has remeshed yet.
        let before = app.doc.totals().resident_bytes;
        let history_before = app.history.stats();

        update(&mut app, Message::MaskAllApplied);
        assert!(app.doc.active_volume().mask().protects_everything());
        assert_eq!(app.doc.totals().resident_bytes, before, "Mask All allocated a map");
        assert_eq!(app.history.stats().bytes, history_before.bytes);
        // The entry's own struct is charged to the reclaim allowance and
        // nothing else: no map, no brick, and nothing that grows with the size
        // of the body it protects.
        let held = app.history.stats().reclaim_bytes - history_before.reclaim_bytes;
        assert!(held < 256, "Mask All over an empty map is holding {held} bytes");
        assert!(app.mask_card.is_some(), "a fully masked body showed no card");

        // Twice is refused rather than pushed as a second no-op entry.
        let entries = app.history.stats().undo_entries;
        update(&mut app, Message::MaskAllApplied);
        assert_eq!(app.history.stats().undo_entries, entries, "Mask All twice pushed two entries");
        assert!(app.status.contains("already"), "the refusal said: {}", app.status);
    }

    /// Mask All over a painted mask throws the map away and inverts, as ONE
    /// change, so undo puts both halves back together.
    #[test]
    fn masking_all_over_a_painted_mask_is_one_entry_that_undoes_whole() {
        let mut app = masked();
        let before = mask_of(&app);
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskAllApplied);
        assert!(app.doc.active_volume().mask().protects_everything());
        assert_eq!(app.history.stats().undo_entries, entries + 1, "Mask All was not one entry");

        update(&mut app, Message::Undo);
        assert!(!app.doc.active_volume().mask().inverted(), "undo left the polarity inverted");
        assert_eq!(mask_of(&app), before, "undo did not put the map back");
    }

    /// **A gesture whose slider went away is committed on the next frame.**
    ///
    /// The amount slider lives in the right-click menu, and Escape closes that
    /// menu -- so the release that commits a drag can simply never arrive. What
    /// would be left is a filtered mask with no history entry and no `unsaved`:
    /// work ctrl+Z cannot reach and that quitting would not prompt about.
    #[test]
    fn a_filter_gesture_orphaned_by_a_closing_menu_is_committed_on_the_next_frame() {
        let mut app = masked();
        app.menu = Some(Vec2::ZERO);
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskAmountChanged(1.0));
        let filtered = mask_of(&app);

        // A frame DURING the drag must not commit it early, or every filter
        // becomes one entry per sixtieth of a second.
        update(&mut app, Message::Frame);
        assert!(app.mask_grab.is_some(), "a frame mid-drag ended the drag");
        assert_eq!(app.history.stats().undo_entries, entries, "a frame mid-drag pushed an entry");

        // The menu goes -- Escape, a press in the viewport, anything.
        app.menu = None;
        update(&mut app, Message::Frame);
        assert!(app.mask_grab.is_none(), "the drag is still open with nothing to release it");
        assert_eq!(app.history.stats().undo_entries, entries + 1, "the orphaned drag was lost");
        assert_eq!(mask_of(&app), filtered, "committing the orphan changed what it committed");
        assert!(app.unsaved, "an orphaned drag left the document looking saved");
        assert_eq!(app.mask_amount, 0.0);

        update(&mut app, Message::Undo);
        assert_ne!(mask_of(&app), filtered, "the committed orphan is not undoable");
    }

    /// **Blur at 0.0 is a no-op: no change, and no entry.**
    ///
    /// Literally rather than approximately: dragging back to zero moves the
    /// snapshot itself back onto the body, so it is the same field object.
    #[test]
    fn a_filter_at_zero_changes_nothing_and_pushes_no_entry() {
        let mut app = masked();
        let before = mask_of(&app);
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskAmountChanged(0.0));
        update(&mut app, Message::MaskAmountReleased);
        assert_eq!(mask_of(&app), before, "a filter at zero moved a value");
        assert_eq!(app.history.stats().undo_entries, entries, "a filter at zero pushed an entry");

        // And out and back within one drag is the same claim about the middle
        // of a gesture rather than the start of one.
        update(&mut app, Message::MaskAmountChanged(0.8));
        update(&mut app, Message::MaskAmountChanged(0.0));
        update(&mut app, Message::MaskAmountReleased);
        assert_eq!(mask_of(&app), before, "dragging out and back did not land where it started");
        assert_eq!(
            app.history.stats().undo_entries,
            entries,
            "dragging out and back pushed an entry"
        );
    }

    /// **The standing card follows a drag rather than freezing on its first
    /// step**, which the mask's own revision counter cannot deliver on its own.
    ///
    /// Every step of an absolute filter is built from ONE snapshot, so each
    /// carries `snapshot.revision + 1` -- the same number for a grow at 0.3 and
    /// a grow at 0.9. The card is keyed on that number. `Volume::replace_mask`
    /// is what moves it past the mask leaving the body; delete that one line
    /// and this test is the only thing that notices.
    #[test]
    fn the_standing_card_follows_a_filter_drag_step_by_step() {
        let mut app = masked();
        update(&mut app, Message::MaskFilterChosen(MaskFilter::Grow));

        update(&mut app, Message::MaskAmountChanged(0.3));
        let light = app.mask_card.clone().expect("a masked body showed no card");
        let light_fill = app.doc.active_volume().mask_fill();
        assert_eq!(
            light.revision,
            app.doc.active_volume().mask().revision(),
            "the card is keyed on a revision the mask is not at"
        );

        update(&mut app, Message::MaskAmountChanged(0.9));
        let heavy = app.mask_card.clone().expect("a masked body showed no card");
        assert_ne!(
            app.doc.active_volume().mask_fill(),
            light_fill,
            "the fixture's two steps produce the same mask, so it proves nothing"
        );
        assert_ne!(
            light.revision, heavy.revision,
            "two steps of one drag share a revision, so the card can never update"
        );
        assert_eq!(
            heavy.revision,
            app.doc.active_volume().mask().revision(),
            "the card froze on the first step of the drag"
        );
    }

    /// **The whole drag is ONE entry, and the same amount twice from the same
    /// grab lands in the same place.**
    ///
    /// This is the absolute gesture. ZBrush's accumulative alternative has been
    /// its top masking complaint since 2007.
    #[test]
    fn blurring_at_one_twice_in_a_drag_equals_blurring_once_and_is_one_entry() {
        let mut app = masked();
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskAmountChanged(1.0));
        let once = mask_of(&app);
        update(&mut app, Message::MaskAmountChanged(1.0));
        assert_eq!(mask_of(&app), once, "the second application of the same amount moved it");
        update(&mut app, Message::MaskAmountReleased);

        assert_eq!(
            app.history.stats().undo_entries,
            entries + 1,
            "one drag became more than one entry"
        );
        assert_eq!(mask_of(&app), once, "the release changed the mask");
        assert_eq!(app.mask_amount, 0.0, "the slider did not go back to zero");
        assert!(app.unsaved);
        assert!(app.status.contains("blurred"), "the status said: {}", app.status);
    }

    /// A blur really blurs -- a softened mask has values the painted one does
    /// not -- and one undo takes the whole drag back off.
    #[test]
    fn a_blur_softens_the_mask_and_one_undo_takes_the_whole_drag_off() {
        let mut app = masked();
        let before = mask_of(&app);

        update(&mut app, Message::MaskAmountChanged(0.4));
        update(&mut app, Message::MaskAmountChanged(1.0));
        update(&mut app, Message::MaskAmountReleased);
        assert_ne!(mask_of(&app), before, "the blur changed nothing at all");

        update(&mut app, Message::Undo);
        assert_eq!(mask_of(&app), before, "one undo did not take the whole drag off");
    }

    /// Choosing a different filter mid-drag commits the drag it interrupts
    /// rather than re-applying one filter's snapshot under another's name.
    #[test]
    fn choosing_another_filter_mid_drag_commits_the_drag_it_interrupts() {
        let mut app = masked();
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskAmountChanged(1.0));
        update(&mut app, Message::MaskFilterChosen(MaskFilter::Grow));
        assert_eq!(app.history.stats().undo_entries, entries + 1, "the interrupted drag was lost");
        assert!(app.mask_grab.is_none(), "the grab outlived its filter");
        assert_eq!(app.mask_amount, 0.0, "the slider kept the old filter's amount");

        update(&mut app, Message::MaskAmountChanged(1.0));
        update(&mut app, Message::MaskAmountReleased);
        assert_eq!(app.history.stats().undo_entries, entries + 2);
        assert!(app.status.contains("grew"), "the status said: {}", app.status);
    }

    /// All four filters run, and each of them changes the mask.
    #[test]
    fn every_one_of_the_four_filters_changes_the_mask() {
        for filter in MaskFilter::ALL {
            let mut app = masked();
            let before = mask_of(&app);
            update(&mut app, Message::MaskFilterChosen(filter));
            update(&mut app, Message::MaskAmountChanged(1.0));
            update(&mut app, Message::MaskAmountReleased);
            assert_ne!(mask_of(&app), before, "{filter:?} did nothing");
        }
    }

    /// **The refusal fires before the walk, and the filter never runs.**
    ///
    /// The peak of a filter is `field + 2 x mask`, which on a fully painted
    /// mask is 1.5 times the field -- 6.22 GiB against a 6 GiB ceiling on the
    /// measured 0.0565 mm dragon, from one button press. `History::trim` keeps
    /// a single over-budget entry by design, so the undo budget cannot save it.
    ///
    /// The document is driven to the ceiling by writing `doc_stats`, which is
    /// the number the guard reads and the only way to reach a six-gigabyte
    /// document in a unit test.
    #[test]
    fn a_filter_that_would_not_fit_is_refused_and_never_runs() {
        let mut app = masked();
        let before = mask_of(&app);
        let entries = app.history.stats().undo_entries;
        app.doc_stats.resident_bytes = MAX_VOLUME_BYTES as usize;

        update(&mut app, Message::MaskAmountChanged(1.0));
        assert_eq!(mask_of(&app), before, "the filter ran after the guard refused it");
        assert!(app.mask_grab.is_none(), "the refusal still took the mask off the body");
        assert_eq!(app.mask_amount, 0.0, "the slider stayed up after the refusal");
        assert_eq!(app.history.stats().undo_entries, entries, "a refused filter pushed an entry");
        assert!(
            app.status.contains("could not blur the mask") && app.status.contains("GB"),
            "the refusal has to name the verb and the cost: {}",
            app.status
        );

        // And a release after a refusal commits nothing.
        update(&mut app, Message::MaskAmountReleased);
        assert_eq!(app.history.stats().undo_entries, entries);
    }

    /// **Clear is NOT gated by the memory guard**, and that is deliberate: it
    /// is the one verb that gives the memory back, and refusing it at the
    /// ceiling would trap a user with a mask they cannot remove.
    #[test]
    fn clear_still_works_with_the_document_at_the_memory_ceiling() {
        let mut app = masked();
        app.doc_stats.resident_bytes = MAX_VOLUME_BYTES as usize;
        update(&mut app, Message::MaskCleared);
        assert!(app.doc.active_volume().mask().is_free(), "Clear was refused at the ceiling");

        let mut app = masked();
        app.doc_stats.resident_bytes = MAX_VOLUME_BYTES as usize;
        update(&mut app, Message::MaskInverted);
        assert!(app.doc.active_volume().mask().inverted(), "Invert was refused at the ceiling");
    }

    /// Clearing a body that carries no mask says so rather than pushing an
    /// entry that restores nothing.
    #[test]
    fn clearing_an_unmasked_body_refuses_by_name() {
        let mut app = app();
        let entries = app.history.stats().undo_entries;
        update(&mut app, Message::MaskCleared);
        assert_eq!(app.history.stats().undo_entries, entries, "Clear on nothing pushed an entry");
        assert!(app.status.contains("no mask"), "the refusal said: {}", app.status);
    }

    /// The two reflex buttons are on the card only when the percentage above
    /// them is about the body they would act on.
    #[test]
    fn the_cards_verbs_appear_only_when_it_names_the_active_body() {
        let mut app = masked();
        let card = app.mask_card.as_ref().expect("a painted mask showed no card");
        assert!(card.names_the_active_body(), "the card is about the body that was masked");

        // A second body, active and unmasked, with the masked one hidden: the
        // card is up for the hidden one and must not offer to clear this one.
        let masked_body = app.doc.active();
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        update(&mut app, Message::BodyVisibilityToggled(masked_body));
        let card = app.mask_card.as_ref().expect("a hidden masked body showed no card");
        assert!(
            !card.names_the_active_body(),
            "the card offered its verbs on a body with no mask: {}",
            card.headline
        );
    }

    /// **A fully protected body VISIBLE on screen is announced even when it is
    /// not the active one.** Increment 24's review finding, inverted into an
    /// assertion.
    ///
    /// The shape of it, as the review reproduced it: add a Cube, press Mask
    /// all, click the other body in the BODIES panel. The active body carries
    /// no mask and the Cube is not hidden, so it was counted nowhere and the
    /// card was set to `None` -- a body that spares every cut and refuses every
    /// stroke, sitting in plain view, with nothing anywhere saying so. The
    /// first the user heard of it was the `bricks spared` status line after
    /// drawing a clip plane.
    ///
    /// The tint is the other half of the signal and it is fixed too -- see
    /// `MeshPool::draw`'s per-bucket polarity -- but the tint is a preference
    /// the user can switch off, and the card is the channel that cannot be. So
    /// this is the assertion that has to hold.
    #[test]
    fn a_fully_masked_body_on_screen_is_announced_even_when_it_is_not_active() {
        let mut app = app();
        let first = app.doc.active();
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let cube = app.doc.active();
        assert_ne!(cube, first, "the primitive did not become the active body");

        update(&mut app, Message::MaskAllApplied);
        assert!(app.doc.active_volume().mask().protects_everything(), "Mask all did nothing");

        // Away to the other body, exactly as clicking its panel row does.
        update(&mut app, Message::BodySelected(first));
        assert_eq!(app.doc.active(), first);
        assert!(app.doc.active_volume().mask().is_free(), "the fixture masked the wrong body");
        assert!(
            app.shown.iter().all(|shown| *shown),
            "the fixture hid something, which is the case the card already covered"
        );

        let card = app.mask_card.as_ref().expect("a fully masked body on screen showed no card");
        assert_eq!(card.headline, "MASK — 1 masked, elsewhere");
        assert!(card.off_screen.is_empty(), "nothing is hidden: {}", card.off_screen);
        assert!(
            !card.names_the_active_body(),
            "the card offered to clear a mask the active body does not have"
        );

        // And it goes when the protection does, rather than standing on stale
        // counts: back to the Cube, Clear, and the document is free again.
        update(&mut app, Message::BodySelected(cube));
        update(&mut app, Message::MaskCleared);
        update(&mut app, Message::BodySelected(first));
        assert!(app.mask_card.is_none(), "the card outlived the mask it was counting");
    }

    /// **Every body that disagrees with the published polarity is named, and
    /// the set follows which body is active.**
    ///
    /// The rendering half of the same finding. `Uniforms::mask_inverted` is one
    /// word for the whole draw and carries the ACTIVE body's polarity, so the
    /// bodies that disagree with it have to be published beside it or they are
    /// drawn through it -- which is how a fully protected body came to be drawn
    /// with no tint. See [`hani_gpu::MaskPolarity`]; the pixels are pinned in
    /// `hani-gpu`'s `offscreen.rs`, which is the only place a pixel exists.
    ///
    /// The assertions are on `app.shared`, which is what the render callback
    /// actually reads, rather than on the application's own kept buffer.
    #[test]
    fn the_bodies_that_disagree_about_polarity_are_published_beside_the_word() {
        let mut app = app();
        let first = app.doc.active();
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let cube = app.doc.active();

        // A document where every body agrees costs the draw nothing.
        assert!(!app.shared.mask_view().inverted);
        assert!(
            app.shared.opposite_polarity_snapshot().is_empty(),
            "an unmasked document published a polarity exception"
        );

        update(&mut app, Message::MaskAllApplied);
        assert!(app.shared.mask_view().inverted, "the active body's polarity did not reach it");
        assert_eq!(
            app.shared.opposite_polarity_snapshot(),
            vec![first],
            "the body that is NOT masked was left to be drawn through the masked one's polarity"
        );

        // The word is the active body's, so choosing another body swaps which
        // side of the document is the exception.
        update(&mut app, Message::BodySelected(first));
        assert!(!app.shared.mask_view().inverted);
        assert_eq!(
            app.shared.opposite_polarity_snapshot(),
            vec![cube],
            "the fully masked body was left to be drawn as free"
        );

        // And undoing the Mask All puts the document back to agreeing.
        update(&mut app, Message::BodySelected(cube));
        update(&mut app, Message::Undo);
        assert!(!app.shared.mask_view().inverted, "undo did not put the polarity back");
        assert!(
            app.shared.opposite_polarity_snapshot().is_empty(),
            "the exception outlived the mask that caused it"
        );
    }
}

/// The GENERATED masks: cavity, smoothness, thickness, the half-space that
/// rides the cut's own drag, and the split that ends the arc.
///
/// Its own module rather than more of `mask_verb_tests`, because what these
/// have to answer for is different: a verb is O(1) and a generator is a pass
/// over the whole body, so the questions are whether it refuses before it runs,
/// whether it pushes an entry when it found nothing, and whether the one
/// gesture it borrows still does what it did.
#[cfg(test)]
mod mask_generator_tests {
    use super::*;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// An application with a real painted mask on its one body.
    ///
    /// The same shape `mask_verb_tests::masked` uses, and deliberately a second
    /// copy rather than a shared helper: that module's fixture is about the
    /// verbs and is free to change with them, and a split that quietly started
    /// testing a different mask would be very hard to see.
    fn masked() -> Hani {
        let mut app = app();
        let centre = Vector::new(SIZE.x / 2.0, SIZE.y / 2.0);
        update(&mut app, Message::ToolChanged(Tool::Mask));
        update(&mut app, Message::BrushRadiusChanged(10.0));
        update(
            &mut app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: centre,
                size: SIZE,
            }),
        );
        for step in 1..=3 {
            update(
                &mut app,
                Message::Pointer(PointerEvent::Moved {
                    position: Vector::new(centre.x + step as f32 * 15.0, centre.y),
                    size: SIZE,
                }),
            );
        }
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
        assert!(app.doc.active_volume().mask_fill() > 0.0, "the fixture painted no mask");
        update(&mut app, Message::ToolChanged(Tool::Sculpt));
        app
    }

    /// Every stored mask byte of the active body, for a bit-for-bit comparison.
    fn mask_of(app: &Hani) -> Vec<(hani_core::BrickCoord, Vec<u8>)> {
        let mask = app.doc.active_volume().mask();
        let mut coords: Vec<_> = mask.brick_coords().collect();
        coords.sort_by_key(|coord| (coord.0.z, coord.0.y, coord.0.x));
        let dim = hani_core::BRICK_DIM as i32;
        coords
            .into_iter()
            .map(|coord| {
                let origin = coord.origin();
                let bytes = (0..dim)
                    .flat_map(|z| {
                        (0..dim).flat_map(move |y| (0..dim).map(move |x| glam::IVec3::new(x, y, z)))
                    })
                    .map(|local| mask.at(origin + local))
                    .collect();
                (coord, bytes)
            })
            .collect()
    }

    /// A checksum of the FIELD, so a generator that quietly wrote a distance is
    /// caught by the tests that say it must not.
    fn field_of(app: &Hani) -> u64 {
        let volume = app.doc.active_volume();
        let mut coords: Vec<hani_core::BrickCoord> = volume.brick_coords().collect();
        coords.sort_by_key(|coord| (coord.0.z, coord.0.y, coord.0.x));
        let dim = hani_core::BRICK_DIM as i32;
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for coord in coords {
            let origin = coord.origin();
            for z in 0..dim {
                for y in 0..dim {
                    for x in 0..dim {
                        let value = volume.sample_voxel(origin + glam::IVec3::new(x, y, z));
                        hash = (hash ^ u64::from(value.to_bits())).wrapping_mul(0x1000_0000_01b3);
                    }
                }
            }
        }
        hash
    }

    /// The default body is a 30 mm sphere, which is smooth everywhere at every
    /// feature size the slider offers -- so this is the generator fixture that
    /// needs no sculpting to produce a mask.
    #[test]
    fn a_smoothness_mask_protects_the_body_and_pushes_exactly_one_entry() {
        let mut app = app();
        let entries = app.history.stats().undo_entries;
        let field = field_of(&app);

        update(&mut app, Message::MaskGenerated(MaskGenerator::Smoothness));

        assert!(!app.doc.active_volume().mask().is_free(), "the smoothness mask protected nothing");
        assert_eq!(app.history.stats().undo_entries, entries + 1, "a generator was not one entry");
        assert!(app.unsaved, "a generated mask did not mark the document unsaved");
        assert!(app.mask_card.is_some(), "a generated mask raised no overlay card");
        assert_eq!(field_of(&app), field, "a generator wrote to the FIELD");
    }

    /// The sign, from the outside. A solid sphere is convex everywhere, so
    /// asking for its cavities has to change nothing at all -- and changing
    /// nothing means no entry and no `unsaved`, not an entry that restores what
    /// is already there.
    #[test]
    fn a_cavity_mask_on_a_convex_sphere_finds_nothing_and_pushes_no_entry() {
        let mut app = app();
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskGenerated(MaskGenerator::Cavity));

        assert!(app.doc.active_volume().mask().is_free(), "a convex sphere was masked as a cavity");
        assert_eq!(app.history.stats().undo_entries, entries, "an empty result pushed an entry");
        assert!(!app.unsaved, "an empty result marked the document unsaved");
        assert!(app.status.contains("nothing"), "the status does not say why: {}", app.status);
    }

    /// A generator REPLACES the mask, and the mask it replaced has to come back
    /// whole -- including its polarity, which a generator always resets.
    #[test]
    fn undoing_a_generated_mask_puts_the_hand_painted_one_back_bit_for_bit() {
        let mut app = masked();
        update(&mut app, Message::MaskInverted);
        let before = mask_of(&app);
        assert!(app.doc.active_volume().mask().inverted());

        update(&mut app, Message::MaskGenerated(MaskGenerator::Smoothness));
        assert!(
            !app.doc.active_volume().mask().inverted(),
            "a generated mask inherited the polarity it was asked to replace"
        );

        update(&mut app, Message::Undo);
        assert_eq!(mask_of(&app), before, "undo did not put the painted mask back");
        assert!(app.doc.active_volume().mask().inverted(), "undo did not put the polarity back");
    }

    /// Nothing thin about a 60 mm ball, and the honest answer to that is a
    /// status line rather than a mask over the whole thing.
    #[test]
    fn a_thickness_mask_on_a_solid_ball_selects_nothing() {
        let mut app = app();
        update(&mut app, Message::MaskGenerated(MaskGenerator::Thickness));
        assert!(app.doc.active_volume().mask().is_free(), "a 60 mm ball was called thin");
    }

    /// The two sliders are session state and both are clamped where they are
    /// received, because a message can arrive from anywhere and the thickness
    /// ceiling is a property of the narrow band rather than a preference.
    #[test]
    fn both_generator_sliders_are_clamped_where_the_message_lands() {
        let mut app = app();

        update(&mut app, Message::MaskThicknessChanged(400.0));
        assert_eq!(app.mask_thickness_voxels, hani_core::MAX_THICKNESS_VOXELS);
        update(&mut app, Message::MaskThicknessChanged(-4.0));
        assert_eq!(app.mask_thickness_voxels, 1, "the thinnest wall is one voxel, never zero");

        update(&mut app, Message::MaskFeatureChanged(-1.0));
        assert_eq!(app.mask_feature_mm, *MASK_FEATURE_RANGE_MM.start());
        update(&mut app, Message::MaskFeatureChanged(1000.0));
        assert_eq!(app.mask_feature_mm, *MASK_FEATURE_RANGE_MM.end());
    }

    /// **The refusal is part of the feature and not a nicety.** A generated
    /// mask writes a value at every surface voxel, so the +25% is what is
    /// actually paid -- and it has to be predicted rather than discovered by an
    /// allocator.
    ///
    /// The ceiling cannot be reached by building a document (six gigabytes of
    /// resident bricks is six gigabytes of real allocation), so what is moved
    /// here is the number the guard is asked about, which is the same thing
    /// `hani_core::split`'s own refusal test does with its guard.
    #[test]
    fn a_generated_mask_that_would_not_fit_is_refused_before_it_runs() {
        let mut app = app();
        // Everything but a few megabytes of the ceiling is already spoken for.
        app.doc_stats.resident_bytes = MAX_VOLUME_BYTES as usize - 4 * 1024 * 1024;
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskGenerated(MaskGenerator::Smoothness));

        assert!(app.doc.active_volume().mask().is_free(), "the refusal did not stop the pass");
        assert_eq!(app.history.stats().undo_entries, entries, "a refusal pushed an entry");
        assert!(!app.unsaved, "a refusal marked the document unsaved");
        assert!(
            app.status.contains("GB of memory") && app.status.contains("mm"),
            "the refusal names neither the ceiling nor a voxel size that would fit: {}",
            app.status
        );
    }

    // --------------------------------------------- the half-space, and its control

    /// Ctrl at the end of a cut drag writes protection instead of taking
    /// material away: the same gesture, the same plane, a different field.
    #[test]
    fn a_ctrl_cut_drag_masks_the_half_instead_of_removing_it() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        let field = field_of(&app);

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        app.control = true;
        let middle_y = SIZE.y / 2.0;
        update(
            &mut app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: Vector::new(SIZE.x * 0.1, middle_y),
                size: SIZE,
            }),
        );
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.9, middle_y),
            size: SIZE,
        });
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));

        assert_eq!(field_of(&app), field, "a ctrl cut removed material");
        assert!(!app.doc.active_volume().mask().is_free(), "a ctrl cut protected nothing");
        assert_ne!(app.tool, Tool::Cut, "the cut stayed armed after being used");
        assert!(app.mask_card.is_some(), "the half-space mask raised no overlay card");

        // The half a plain cut would have removed is the half that is protected,
        // and its opposite is free -- which is the sentence the doc comment
        // makes and the only part of it a number can hold up.
        let voxel = app.doc.voxel_size();
        let protection = |mm: f32| {
            let cell = (Vec3::new(0.0, mm, 0.0) / voxel).round().as_ivec3();
            app.doc.active_volume().mask().at(cell)
        };
        assert_ne!(protection(12.0) > 128, protection(-12.0) > 128, "both halves read the same");
    }

    /// The control for the test above, in the same module: without ctrl the cut
    /// still cuts, so the branch cannot be reached by a modifier that is stuck
    /// on.
    #[test]
    fn a_cut_drag_without_ctrl_still_removes_material() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        let field = field_of(&app);

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        let middle_y = SIZE.y / 2.0;
        update(
            &mut app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: Vector::new(SIZE.x * 0.1, middle_y),
                size: SIZE,
            }),
        );
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.9, middle_y),
            size: SIZE,
        });
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));

        assert_ne!(field_of(&app), field, "a plain cut removed nothing");
        assert!(app.doc.active_volume().mask().is_free(), "a plain cut painted a mask");
    }

    // --------------------------------------------------- splitting the mask off

    #[test]
    fn splitting_off_the_mask_makes_two_bodies_and_one_undo_puts_the_source_back() {
        let mut app = masked();
        let source = app.doc.active();
        let bodies = app.doc.body_count();
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::BodySplitMasked);

        assert_eq!(app.doc.body_count(), bodies + 1, "a masked split did not make a second body");
        assert!(app.doc.index_of(source).is_none(), "the source was not consumed");
        assert_eq!(app.history.stats().undo_entries, entries + 1, "a split was not one entry");
        assert!(app.unsaved);
        assert!(
            app.doc.active_volume().mask().is_free(),
            "the half the user was left on arrived carrying a mask"
        );

        update(&mut app, Message::Undo);
        assert_eq!(app.doc.body_count(), bodies, "one undo did not take both halves back");
        assert!(app.doc.index_of(source).is_some(), "undo did not restore the source row");
        assert!(
            !app.doc.active_volume().mask().is_free(),
            "undo restored the geometry but not the mask that chose the split"
        );
    }

    /// The refusal names the body, because a document with sixty-four rows in it
    /// makes "carries no mask" a question about which one.
    #[test]
    fn splitting_off_a_mask_that_is_not_there_is_refused_by_name() {
        let mut app = app();
        let bodies = app.doc.body_count();
        update(&mut app, Message::BodySplitMasked);
        assert_eq!(app.doc.body_count(), bodies, "an unmasked body was split anyway");
        assert!(app.status.contains("no mask"), "the refusal does not say why: {}", app.status);
        assert!(!app.unsaved, "a refusal marked the document unsaved");
    }
}
