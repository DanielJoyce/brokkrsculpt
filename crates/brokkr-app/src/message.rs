// SPDX-License-Identifier: AGPL-3.0-only

//! Messages exchanged between the widget tree and the application.

use brokkr_core::{BrushKind, FalloffCurve, MaskFilter, MirrorAxis, PatternKind};
use iced::Vector;

use crate::app::{SizingTarget, Tool};
use crate::spacemouse::{Action, Axis, ButtonAction, Mode};

/// A file format the sculpt can be written to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Binary STL. Universal, carries no units, repeats every vertex.
    Stl,
    /// Wavefront OBJ. Text, shares vertices, carries normals.
    Obj,
    /// 3MF. The only one of the three that states its units.
    ThreeMf,
    /// PLY with a colour at every vertex: what a painted model looks like,
    /// for a full colour process, where a 3MF says which filament prints
    /// what. See `brokkr_core::export::ply`.
    Ply,
    /// OBJ with a material file and a picture for each body: the paint
    /// baked into a texture, so that a small mesh still looks like the
    /// model. See `brokkr_core::export::texture`.
    Textured,
    /// Binary glTF: the same mesh and the same pictures as [`Self::Textured`],
    /// in one file, in metres and Y up as that format is. See
    /// `brokkr_core::export::glb`.
    Glb,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 6] = [
        ExportFormat::Stl,
        ExportFormat::Obj,
        ExportFormat::ThreeMf,
        ExportFormat::Ply,
        ExportFormat::Textured,
        ExportFormat::Glb,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Stl => "STL",
            ExportFormat::Obj => "OBJ",
            ExportFormat::ThreeMf => "3MF",
            ExportFormat::Ply => "PLY",
            ExportFormat::Textured => "OBJ+PNG",
            ExportFormat::Glb => "GLB",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Stl => "stl",
            ExportFormat::Obj => "obj",
            ExportFormat::ThreeMf => "3mf",
            ExportFormat::Ply => "ply",
            ExportFormat::Textured => "obj",
            ExportFormat::Glb => "glb",
        }
    }

    /// Whether the format carries the paint as a picture, and so has a
    /// texture size to choose.
    pub fn has_texture(self) -> bool {
        matches!(self, ExportFormat::Textured | ExportFormat::Glb)
    }
}

/// How finely an export is meshed.
///
/// A tolerance in millimetres, named for what it is good for, because that is
/// the question somebody exporting is asking. A percentage of triangles says
/// nothing about a print; "no point of this file is further than a
/// hundredth of a millimetre from what you sculpted" does. See
/// `brokkr_core::export::decimate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportDetail {
    /// Every triangle the mesher made: one vertex a voxel. What an export was
    /// before there was a choice, and still the default, so that nothing
    /// about an existing workflow changes until this is touched.
    #[default]
    Full,
    /// Within 0.01 mm. Finer than a resin printer's pixel.
    Fine,
    /// Within 0.03 mm. Well under what a 0.4 mm nozzle resolves.
    Standard,
    /// Within 0.1 mm. For a quick look in a slicer.
    Draft,
}

impl ExportDetail {
    pub const ALL: [ExportDetail; 4] =
        [ExportDetail::Full, ExportDetail::Fine, ExportDetail::Standard, ExportDetail::Draft];

    /// The tolerance to decimate to, or `None` to leave the mesh as it is.
    ///
    /// The decimator clamps this to one voxel, so on a coarse sculpt Draft and
    /// Standard can come to the same thing. The status line reports the
    /// tolerance that was actually used.
    pub fn tolerance_mm(self) -> Option<f32> {
        match self {
            ExportDetail::Full => None,
            ExportDetail::Fine => Some(0.01),
            ExportDetail::Standard => Some(0.03),
            ExportDetail::Draft => Some(0.1),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ExportDetail::Full => "Full (every triangle)",
            ExportDetail::Fine => "Fine (0.01 mm)",
            ExportDetail::Standard => "Standard (0.03 mm)",
            ExportDetail::Draft => "Draft (0.1 mm)",
        }
    }
}

impl std::fmt::Display for ExportDetail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// One setting of the key a parting puts between two parts.
///
/// One of the form tools' controls: thicken or thin a body, soften or
/// exaggerate its detail. The three settings change nothing in a document;
/// the two verbs rebuild the active body, each as one undo entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FormMessage {
    /// What counts as detail, in millimetres.
    DetailSizeChanged(f32),
    /// How far a bulk moves the surface, in millimetres. Negative thins.
    AmountChanged(f32),
    /// What the detail is multiplied by.
    GainChanged(f32),
    BulkApplied,
    DetailGainApplied,
}

/// The revolve's controls. See `app/revolve.rs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RevolveMessage {
    /// Put the axis up for a line to be drawn, or away.
    Armed(bool),
}

/// The deformers' controls. See `app/deform.rs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeformMessage {
    Kind(brokkr_core::deform::DeformKind),
    Axis(brokkr_core::deform::Axis),
    /// Degrees for a twist or a bend, the far end's scale for a taper.
    Amount(f32),
    Applied,
}

/// The shell and extract controls. See `app/hollow.rs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HollowMessage {
    /// A shell's wall, in millimetres.
    WallChanged(f32),
    ShellApplied,
    /// An extracted plate's thickness, in millimetres.
    ThicknessChanged(f32),
    /// The share of the thickness that goes into the body, nought to one.
    InwardChanged(f32),
    /// How far the plate is moved out along the normal, in millimetres.
    OffsetChanged(f32),
    ExtractApplied,
}

/// One change to the studio brushes, on [`KeySetting`]'s pattern: one
/// message with a payload and not three entries in the message table.
/// Settings of the application and not of the document.
#[derive(Debug, Clone, PartialEq)]
pub enum StudioMessage {
    /// What the brush in hand does to the surface: an amount of each of
    /// the five things a brush can do. Amounts that are just a classic
    /// brush's are that classic brush.
    ActionsChosen(brokkr_core::brush::compound::Actions),
    /// Give the stroke the cross-section of a ball, which no falloff curve
    /// of the classic brush's is. Choosing one of those curves again
    /// (`Message::FalloffChanged`) puts it away.
    RoundChosen,
    /// How much of the middle of the stroke is flat, as a share of its
    /// radius.
    PlateauChanged(f32),
    /// Show what the brush in hand is made of, to change it, or put that
    /// away.
    SettingsShown(bool),
    /// Bring the palette of brushes up at the pointer, or put it away if
    /// it is up. The `b` key.
    PaletteToggled,
    /// Ask for brush files to take in.
    BrushImportRequested,
    /// The files that were picked, each read: its name and its text, or
    /// why it could not be read. `None` when the dialog was dismissed.
    BrushImported(Option<Vec<Result<(String, String), String>>>),
    /// Read the brushes directory again: for brushes put there by hand
    /// while the application was running.
    BrushesReloaded,
    /// Lay the picture in hand across the tip of the brush, or take it
    /// off. The picture is the one the stamp presses: one of the six that
    /// come with the application, or any picture.
    TipToggled(bool),
    /// Whether the brush builds up, every stamp on the last, as a brush
    /// always has; or, `false`, a stroke lays one layer however long it
    /// dwells and however often it crosses itself.
    BuildUpToggled(bool),
    /// Set the brush down this many of its own widths apart along the
    /// stroke, for separate dabs. Nothing, or less, is a line.
    DabsChanged(f32),
    /// How far the brush trails the pointer, in pixels. Zero is off.
    LazyChanged(f32),
    /// How much of the brush's size pen pressure decides, 0..=1.
    SizeFromPressureChanged(f32),
    /// Take up the stamp, or put it away. With it up, a drag on the model
    /// presses the stamp image in: press where it goes, drag to size and
    /// turn it.
    StampToggled(bool),
    /// Use one of the stamp images that come with the application.
    StampChosen(brokkr_core::brush::alpha::BuiltIn),
    /// How tall the stamp is, as a share of its size.
    StampHeightChanged(f32),
    /// Lay the stamp again and again along the drag, this many of its own
    /// widths apart, each at the brush's size. Nothing, or less, is one
    /// stamp sized by the drag.
    StampAlongChanged(f32),
    /// Ask for a picture to use as the stamp image.
    StampImageRequested,
    /// The picture that was chosen, read and turned into heights: its name
    /// and the image, or why it could not be used. `None` when the dialog
    /// was dismissed.
    StampImageLoaded(Option<Result<LoadedStamp, String>>),
    /// Take up a brush from the palette: every setting it holds, at once.
    /// The number is its place on the palette.
    PresetChosen(usize),
    /// Open or close the block at the top of the palette that adds a brush:
    /// a name for the one in hand, and brush files taken in.
    AddToggled,
    /// The name being typed for a brush about to be kept.
    PresetNamed(String),
    /// Keep the brush as it is set up now under the name typed, in place
    /// of the brush with that name if there is one.
    PresetSaved,
    /// Forget a brush the user kept. The number is its place on the
    /// palette. One that came with the application stays; one kept under
    /// the name of one that did is forgotten, and that one comes back.
    PresetRemoved(usize),
}

/// The maps a brush that adds can stamp. See `app/maps.rs`.
#[derive(Debug, Clone, PartialEq)]
pub enum MapsMessage {
    /// Ask for a map file to take in.
    Requested,
    /// The file was read and kept, or why not, or the dialog was
    /// dismissed.
    Taken(Option<Result<crate::app::MapName, String>>),
}

/// The controls of the brushes that add. See `app/adding.rs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AddMessage {
    /// Take up something to add (a shape, a body lent as one, a cable), or
    /// with `None` go back to an ordinary brush.
    Chosen(Option<crate::app::Insert>),
    /// Leave what is added as a body of its own, or bake it into the body
    /// in hand.
    SeparateToggled(bool),
    /// How soft the join to the body is, in voxels.
    BlendChanged(f32),
    /// How far off the surface it is held.
    LiftChanged(f32),
    /// What is at the start of a cable, and at its end.
    StartChosen(brokkr_core::solid::End),
    EndChosen(brokkr_core::solid::End),
    /// How far a hung cable sags, as a share of its span.
    SagChanged(f32),
}

/// The workbench's controls. See `app/workbench.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchMessage {
    /// Set the document aside and put a piece to try a brush on in its
    /// place.
    Opened(crate::app::Piece),
    /// Throw the piece away for a fresh one.
    Fresh,
    /// Put the document back.
    Left,
}

/// The measure's controls. See `app/measure.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureMessage {
    /// Begin a measurement, the next two left clicks on the model being its
    /// two points, or with `false` put the one being taken away.
    Armed(bool),
    /// Clear what was measured.
    Cleared,
}

/// How a colour combines with what is under it. See
/// [`brokkr_core::paint::stroke::Mode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaintMode {
    #[default]
    Paint,
    Glaze,
    Erase,
}

impl PaintMode {
    pub const ALL: [PaintMode; 3] = [PaintMode::Paint, PaintMode::Glaze, PaintMode::Erase];

    pub fn label(self) -> &'static str {
        match self {
            PaintMode::Paint => "Paint",
            PaintMode::Glaze => "Glaze",
            PaintMode::Erase => "Erase",
        }
    }
}

/// The colour brush's panel. See `app/painting.rs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PaintingMessage {
    /// The colour in hand, whole, as a swatch gives it.
    Colour([u8; 3]),
    /// One channel of it, as a slider gives it.
    Channel(usize, u8),
    /// How much of itself a full touch of the brush lays.
    Opacity(f32),
    Mode(PaintMode),
    /// How unlike the paint it began on a fill of colour will still run
    /// over, in levels of 255.
    FillLike(u8),
    /// Arm the eyedropper, or put it down again.
    PickArmed,
    /// Coat the whole body in the colour in hand.
    Coated,
    /// How strongly washes and contrasts pool, on every body. See
    /// `app/pooling.rs`.
    Pooling(f32),
    /// The pooling slider let go: keep it.
    PoolingKept,
}

/// A change to the active body's layers of paint. See `app/layers.rs`.
/// Each names a layer by its place in the stack, bottom being 0.
#[derive(Debug, Clone, PartialEq)]
pub enum LayersMessage {
    /// A new layer over the active one.
    Added,
    /// Paint on this one.
    Chosen(usize),
    Shown(usize),
    Locked(usize),
    Blended(usize, brokkr_core::paint::layers::Blend),
    Opacity(usize, f32),
    Renamed(usize, String),
    /// One place up the stack, or down.
    Raised(usize),
    Lowered(usize),
    /// A copy over it, made active.
    Duplicated(usize),
    MergedDown(usize),
    Deleted(usize),
    /// Every layer into one.
    Flattened,
}

/// How the paint index is being searched. See `app/rack.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchBy {
    /// By the words of a name, a brand, a line or a code.
    #[default]
    Name,
    /// By the paints nearest the colour in hand.
    Colour,
}

/// Which colour of a coat a swatch is. See `app/coats.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoatColour {
    Dark,
    Middle,
    Bright,
    ChopDark,
    ChopMiddle,
    ChopBright,
    Contrast,
}

/// A change to the one-click coats. See `app/coats.rs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CoatsMessage {
    Opened,
    /// The light from above, or from where the camera is.
    Above,
    FromView,
    Azimuth(f32),
    Height(f32),
    /// Give this colour of a coat the colour in hand.
    Take(CoatColour),
    /// A zenithal's middle primer, on or off.
    Middle(bool),
    Cone(f32),
    Shadows(bool),
    /// A spongechop's contrast, on or off.
    Contrast(bool),
    BaseCoat,
    Zenithal,
    Spongechop,
    /// The finish a varnish lays.
    Varnish(brokkr_core::paint::Finish),
    VarnishCoat,
}

/// Which picture a decal is. See `app/decal.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecalPick {
    BuiltIn(brokkr_core::paint::decal::BuiltInPicture),
    /// One of the user's own, by its place.
    Own(usize),
}

/// A change to the Decal touch. See `app/decal.rs`.
#[derive(Debug, Clone)]
pub enum DecalMessage {
    Chosen(DecalPick),
    /// As a stamp in the paint in hand, or in its own colours.
    AsStamp(bool),
    Soft(f32),
    /// Ask for a picture.
    TakeIn,
    TakenIn(Option<Result<(String, std::sync::Arc<brokkr_core::paint::decal::Picture>), String>>),
}

/// A change to the airbrush. See `app/airbrush.rs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AirbrushMessage {
    /// How much of a coat a second lays at the middle of the spot.
    Flow(f32),
    Overspray(bool),
}

/// A change to the racks of paints, or to the search that fills them.
/// See `app/rack.rs`. A paint is named by its place on the rack in hand,
/// or in the index.
#[derive(Debug, Clone, PartialEq)]
pub enum RackMessage {
    /// Take this rack in hand.
    Chosen(usize),
    New,
    Duplicated,
    Deleted,
    Renamed(String),
    /// Take this paint of the rack into the brush.
    Picked(usize),
    Raised(usize),
    Lowered(usize),
    Removed(usize),
    /// Open the index search, or shut it.
    Searching,
    Query(String),
    By(SearchBy),
    KindFilter(Option<crate::paint_index::Kind>),
    BrandFilter(Option<&'static str>),
    /// Take a paint of the index into the brush, without putting it on
    /// the rack.
    Tried(usize),
    /// Put a paint of the index on the rack.
    Added(usize),
}

/// The ring cut. See `app/ring_cut.rs`.
#[derive(Debug, Clone, PartialEq)]
pub enum RingMessage {
    /// Read the cut tool's drag as a ring or a line.
    Way(crate::app::CutWay),
    /// Split the piece the proposed cut severs into a body of its own.
    Split,
    /// The same, with pegs and sockets between the two.
    Part,
    /// Drop the proposed cut.
    Dropped,
    /// How much of the rest of the model is screened while a cut is
    /// proposed, 0..=1.
    SeeThrough(f32),
    /// Put a bent cut's points back on its plane.
    Flat,
}

/// The worker threads, in Preferences. See `app/cores.rs`.
#[derive(Debug, Clone, PartialEq)]
pub enum CoresMessage {
    Chosen(crate::app::ThreadsChoice),
    /// Time the work with each count of threads.
    Measure,
    Measured(Vec<brokkr_core::calibrate::Trial>),
}

/// A change in the paint designer, the card a paint is made or changed
/// in. See `app/designer.rs`.
#[derive(Debug, Clone, PartialEq)]
pub enum DesignerMessage {
    /// Open it on the colour in hand, for a new paint.
    New,
    /// Open it on this paint of the rack in hand, to change it.
    Edit(usize),
    Close,
    /// Put the paint on the rack and take it up.
    Save,
    Name(String),
    Kind(crate::paint_index::Kind),
    /// Start from this colour, with nothing mixed in.
    Colour([u8; 3]),
    /// One channel of the colour, red 0, green 1, blue 2.
    Channel(usize, u8),
    Hex(String),
    /// Mix this colour in, or nothing.
    MixWith(Option<[u8; 3]>),
    /// How much of it, 0 to 1.
    Mix(f32),
}

/// The overhang view. See `app/overhang.rs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverhangMessage {
    /// Show where the model, printed as it stands, would need support.
    Shown(bool),
    /// The steepest lean from upright the printer manages, in degrees.
    Steepest(f32),
}

/// A stamp image read from a file.
///
/// Behind an `Arc` because a message is cloned on its way through the
/// runtime and an image is up to a megabyte of heights.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedStamp {
    pub name: String,
    pub alpha: std::sync::Arc<brokkr_core::brush::alpha::Alpha>,
}

/// How the paint brush is put to the model. See
/// [`brokkr_core::paint::touch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaintTouch {
    /// Everything under the brush: a base coat, or detail.
    #[default]
    Loaded,
    /// Only what stands up into a nearly dry brush.
    Drybrush,
    /// A speckle, as a dabbed sponge leaves.
    Sponge,
    /// Not a stroke at all: the paint is poured where the pointer is and
    /// runs until a line of the model stops it.
    Fill,
    /// A wash: laid where the brush goes, on a layer of the Wash blend,
    /// which pools it in the recesses by how shut in they are.
    Wash,
    /// A contrast paint: laid where the brush goes, on a layer of the
    /// Contrast blend, which tints by what is under it and pools.
    Contrast,
    /// Sprayed along the view while the button is held.
    /// See `app/airbrush.rs`.
    Airbrush,
    /// A picture dabbed on. See `app/decal.rs`.
    Decal,
}

impl PaintTouch {
    pub const ALL: [PaintTouch; 8] = [
        PaintTouch::Loaded,
        PaintTouch::Wash,
        PaintTouch::Contrast,
        PaintTouch::Drybrush,
        PaintTouch::Sponge,
        PaintTouch::Fill,
        PaintTouch::Airbrush,
        PaintTouch::Decal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PaintTouch::Wash => "Wash",
            PaintTouch::Contrast => "Contrast",
            PaintTouch::Loaded => "Loaded",
            PaintTouch::Drybrush => "Drybrush",
            PaintTouch::Sponge => "Sponge",
            PaintTouch::Fill => "Fill",
            PaintTouch::Airbrush => "Airbrush",
            PaintTouch::Decal => "Decal",
        }
    }
}

/// One setting of the drybrush or the sponge, on [`KeySetting`]'s pattern.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TouchSetting {
    /// How lightly the drybrush has to touch to leave paint.
    DryCatch(f32),
    /// How broken into bristle lines its marks are.
    DryStreak(f32),
    /// How wide one bristle's mark is, in millimetres.
    DryBristle(f32),
    /// How far the drybrush goes before it runs out, in millimetres. Zero is
    /// "never".
    DryLoad(f32),
    /// The share of a flat a sponge covers.
    SpongeDensity(f32),
    /// The size of one speck, in millimetres.
    SpongeGrain(f32),
    /// How much more of it lands on edges.
    SpongeEdges(f32),
    /// How far into a recess it reaches.
    SpongeReach(f32),
    /// How much it prefers what faces the light.
    SpongeLight(f32),
    SpongeFoam(brokkr_core::paint::touch::Foam),
    /// Whether a fill stops at creases, and at edges.
    FillStopsAtCreases(bool),
    FillStopsAtEdges(bool),
    /// How sharp a line has to be to stop one, as a share of a full crease.
    FillSharpness(f32),
}

/// One message with a payload, on [`SpaceMouseSetting`]'s pattern, so that
/// four controls are four values of one thing and not four entries in the
/// message table. Settings of the application and not of the document: the
/// clearance in particular belongs to the printer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KeySetting {
    Kind(brokkr_core::part::KeyKind),
    /// Radius at the cut face, in millimetres.
    Radius(f32),
    /// How far it reaches into the second part, in millimetres.
    Depth(f32),
    /// The gap left per side, in millimetres.
    Clearance(f32),
}

/// Which pointer button an event refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
}

/// Input from the viewport, in widget local pixels.
#[derive(Debug, Clone, Copy)]
pub enum PointerEvent {
    Moved {
        position: Vector,
        size: Vector,
    },
    Pressed {
        button: PointerButton,
        position: Vector,
        size: Vector,
    },
    Released {
        button: PointerButton,
    },
    /// Positive is a scroll toward the model.
    ///
    /// Carries where the pointer was, because the zoom is anchored on what is
    /// under it rather than on the camera's target. Legal to add without
    /// weakening the capture rule: the scroll arm already gates on
    /// `cursor.position_in(bounds)`, so there is no case where a scroll is
    /// routed here without a position to go with it.
    Scrolled {
        amount: f32,
        position: Vector,
        size: Vector,
    },
    Modifiers {
        shift: bool,
        control: bool,
        alt: bool,
    },
}

/// A menu on the bar along the top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopMenu {
    File,
    Edit,
    View,
    Help,
}

impl TopMenu {
    pub const ALL: [TopMenu; 4] = [TopMenu::File, TopMenu::Edit, TopMenu::View, TopMenu::Help];

    pub fn label(self) -> &'static str {
        match self {
            TopMenu::File => "File",
            TopMenu::Edit => "Edit",
            TopMenu::View => "View",
            TopMenu::Help => "Help",
        }
    }
}

/// A button of the tool rail down the left of the window. See
/// `app/rail.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rail {
    /// Whichever brush of the palette is in hand.
    Brush,
    /// Transform's: a part to add is in hand, and a drag sets it down as a
    /// body of its own.
    Place,
    /// A click on the model is a point of a measurement.
    Measure,
    /// Upstream's Move: a drag carries the surface along.
    Move,
    /// The paint brush.
    Paint,
    Mask,
    Transform,
    Cut,
}

impl Rail {
    pub const ALL: [Rail; 8] = [
        Rail::Brush,
        Rail::Place,
        Rail::Move,
        Rail::Paint,
        Rail::Mask,
        Rail::Transform,
        Rail::Cut,
        Rail::Measure,
    ];
}

/// A workspace of the window: what the rail, the bar and the dock offer.
/// See `app/workspace.rs` and `docs/WORKSPACES.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Workspace {
    #[default]
    Sculpt,
    Transform,
    Paint,
    Print,
}

impl Workspace {
    pub const ALL: [Workspace; 4] =
        [Workspace::Sculpt, Workspace::Transform, Workspace::Paint, Workspace::Print];

    pub fn label(self) -> &'static str {
        match self {
            Workspace::Sculpt => "Sculpt",
            Workspace::Transform => "Transform",
            Workspace::Paint => "Paint",
            Workspace::Print => "Print",
        }
    }

    /// Its key, as the switcher's tooltip says it.
    pub fn key(self) -> &'static str {
        match self {
            Workspace::Sculpt => "Ctrl 1",
            Workspace::Transform => "Ctrl 2",
            Workspace::Paint => "Ctrl 3",
            Workspace::Print => "Ctrl 4",
        }
    }
}

/// The two tabs of the dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockTab {
    /// What the workspace offers to pick from.
    #[default]
    Library,
    /// The settings of what is in hand.
    Inspector,
}

impl DockTab {
    /// The tab's name in a workspace: its library is named for what is in
    /// it.
    pub fn label(self, workspace: Workspace) -> &'static str {
        match (self, workspace) {
            (DockTab::Inspector, _) => "Settings",
            (DockTab::Library, Workspace::Sculpt) => "Brushes",
            (DockTab::Library, Workspace::Transform) => "Parts",
            (DockTab::Library, Workspace::Paint) => "Paints",
            (DockTab::Library, Workspace::Print) => "Export",
        }
    }
}

/// What the user answered to the unsaved-work prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmChoice {
    /// Write the file first, then go on with whatever was asked for.
    Save,
    /// Throw the changes away and go on.
    Discard,
    /// Stay where we are and do nothing.
    Cancel,
}

/// A finished import, on its way from a background task to the update loop.
///
/// Hand written rather than a `Box` or a plain field, and the reason is not
/// stylistic: `Message` derives `Debug` and `Clone`, `Volume` derives neither,
/// and `Box<T>` is `Clone` only when `T` is. `Arc<Mutex<T>>` is `Clone` for any
/// `T` but is `Debug` only when `T` is, so the `Debug` impl below is what makes
/// the pair work. Do not "simplify" this into a `Box` -- it will not compile.
///
/// The slot is taken from rather than read, because the volume moves out into
/// the application exactly once.
#[derive(Clone)]
pub struct ImportPayload(
    pub std::sync::Arc<std::sync::Mutex<Option<Result<Imported, brokkr_core::import::ImportError>>>>,
);

impl ImportPayload {
    pub fn new(result: Result<Imported, brokkr_core::import::ImportError>) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(Some(result))))
    }

    /// Take the result out. A second call yields `None`, which is correct: a
    /// message can in principle be delivered twice and the volume must move
    /// once.
    pub fn take(&self) -> Option<Result<Imported, brokkr_core::import::ImportError>> {
        self.0.lock().ok().and_then(|mut slot| slot.take())
    }
}

impl std::fmt::Debug for ImportPayload {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str("ImportPayload(..)")
    }
}

/// What a successful import produced.
pub struct Imported {
    pub volume: brokkr_core::Volume,
    pub report: brokkr_core::voxelise::VoxeliseReport,
    pub source: std::path::PathBuf,
    pub elapsed_ms: f64,
    /// Which way the mesh's own up pointed, if it could be guessed, already
    /// expressed in sculpt space. See [`brokkr_core::resting_up`].
    ///
    /// Carried from the import task because it has to be measured on the mesh,
    /// and by the time the volume exists the mesh is gone.
    pub resting_up: Option<brokkr_core::Facing>,
}

/// A collapsible block of the properties panel.
///
/// The panel has more in it than fits a 1080 high window, and a scrollable
/// alone left every settings section below the fold — including the SpaceMouse
/// bindings, which made a deliberately rebindable puck practically
/// unrebindable. Collapsing keeps every heading reachable without a scroll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelSection {
    Bodies,
    Brushes,
    Workbench,
    Add,
    Brush,
    Stroke,
    Form,
    Extract,
    Deform,
    Revolve,
    Pattern,
    History,
    Detail,
    Filament,
    Export,
}

impl PanelSection {
    /// In the order the panel draws them.
    pub const ALL: [PanelSection; 15] = [
        PanelSection::Bodies,
        PanelSection::Brushes,
        PanelSection::Workbench,
        PanelSection::Add,
        PanelSection::Brush,
        PanelSection::Stroke,
        PanelSection::Form,
        PanelSection::Extract,
        PanelSection::Deform,
        PanelSection::Revolve,
        PanelSection::Pattern,
        PanelSection::History,
        PanelSection::Detail,
        PanelSection::Filament,
        PanelSection::Export,
    ];

    pub fn title(self) -> &'static str {
        match self {
            PanelSection::Bodies => "BODIES",
            PanelSection::Brushes => "BRUSHES",
            PanelSection::Workbench => "WORKBENCH",
            PanelSection::Add => "ADD",
            PanelSection::Brush => "BRUSH",
            PanelSection::Stroke => "STROKE",
            PanelSection::Form => "FORM",
            PanelSection::Extract => "EXTRACT",
            PanelSection::Deform => "DEFORM",
            PanelSection::Revolve => "REVOLVE",
            PanelSection::Pattern => "PATTERN",
            PanelSection::History => "HISTORY",
            PanelSection::Detail => "DETAIL",
            PanelSection::Filament => "FILAMENT",
            PanelSection::Export => "EXPORT",
        }
    }

    /// Whether it starts open. The long ones start closed so every heading is
    /// visible without scrolling.
    ///
    /// **DETAIL joined that list when BODIES arrived, and it is the sixth
    /// section that pays for it.** The budget is a 1080 high window with every
    /// heading reachable, and the handoff already reports DETAIL's advice line
    /// clipped below the fold at that height -- so it was the block already
    /// costing more than it showed.
    ///
    /// FILAMENT is the seventh, and starts closed for the same reason: it is
    /// four rows plus a button, it sits directly above EXPORT which must stay
    /// reachable, and it is read at the moment somebody exports rather than
    /// while they sculpt.
    ///
    /// FORM starts closed since every heading folds (`ui.rs`): it works on
    /// the whole body, which is done now and then and not stroke by stroke.
    pub fn open_by_default(self) -> bool {
        !matches!(
            self,
            PanelSection::Detail
                | PanelSection::Filament
                | PanelSection::Form
                | PanelSection::Extract
        )
    }
}

/// One change to the SpaceMouse settings.
///
/// Gathered into one message rather than a variant each, because there are
/// twelve of them and they all do the same thing: patch the config and, for
/// the ones that are not mid drag, write it out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpaceMouseSetting {
    Mode(Mode),
    Deadzone(f32),
    PanSens(f32),
    ZoomSens(f32),
    OrbitSens(f32),
    /// Which raw axis drives an action.
    Binding(Action, Axis),
    Invert(Action, bool),
    Button(usize, ButtonAction),
    /// Flip every axis at once.
    InvertAll,
    /// Back to the built in defaults.
    Reset,
    /// Persist the current settings. Sent when a slider is released rather
    /// than on every step of a drag, which would rewrite the file sixty times
    /// a second.
    Save,
}

/// Not `Copy`: one variant carries the raw text of a numeric field being typed
/// into, which has to be owned so a half typed value survives.
#[derive(Debug, Clone)]
pub enum Message {
    Pointer(PointerEvent),
    /// The pointer moved over the timeline strip, at this offset in pixels
    /// from its left edge.
    ///
    /// Carried separately from `Pointer` because it is a different widget with
    /// its own coordinate space, and because `mouse_area`'s press carries no
    /// position at all -- this is what a press then acts on.
    TimelineHover(f32),
    TimelinePressed,
    TimelineReleased,
    TimelineLeft,
    /// Remove the key under the pointer.
    TimelineRemoveKey,
    TimelinePlayToggled,
    /// The strip was laid out this many pixels wide.
    TimelineResized(f32),

    /// Export the sculpt to a staging file and open it in OrcaSlicer.
    OpenInSlicer,
    /// Ask the configured printer what it is doing.
    PrinterChecked,
    /// What it said.
    PrinterAnswered(Result<String, String>),
    /// Ask the printer which filament is loaded, and take it into the palette.
    PaletteSyncRequested,
    /// What it said. `Ok(None)` is a printer that does not report filament,
    /// which is a different thing from one that could not be reached.
    PaletteSynced(Result<Option<Vec<crate::printer::Filament>>, String>),

    /// Open the bug report dialog.
    BugReportOpened,
    /// The description editor did something.
    BugReportEdited(iced::widget::text_editor::Action),
    /// Include the session trail and diagnostics, or do not.
    BugReportDetailToggled(bool),
    /// Send it to TinkerAtlas.
    BugReportSubmitted,
    /// The send finished, with what to show for it.
    BugReportFinished(Result<String, String>),
    /// Put the exact payload on the clipboard instead of sending it.
    BugReportCopied,
    BugReportDismissed,
    /// One presented frame, used to drive the frame rate readout and to keep
    /// the viewport redrawing while a stroke is in progress.
    Frame,
    BrushKindChanged(BrushKind),
    BrushRadiusChanged(f32),
    BrushStrengthChanged(f32),
    FalloffChanged(FalloffCurve),
    /// Which surface pattern multiplies into the brush. See
    /// `brokkr_core::pattern`: one control that modifies every brush, rather
    /// than a brush per pattern.
    PatternChanged(PatternKind),
    PatternScaleChanged(f32),
    PatternDepthChanged(f32),
    /// Flip one mirror plane. A toggle rather than an explicit on/off,
    /// because both the strip button and the keyboard act on what is currently
    /// set rather than knowing it.
    SymmetryAxisToggled(MirrorAxis),
    /// Multiply the brush radius, for the keyboard nudge. Multiplicative
    /// because the radius spans fifty to one and a fixed step would crawl at
    /// the top and jump at the bottom.
    BrushRadiusScaled(f32),
    /// A hold-and-drag adjustment of one brush number has begun. ZBrush's `s`
    /// and `u`: the fast path for radius in every sculpting tool is a drag
    /// against a live ring, not a slider in a panel.
    SizingStarted(SizingTarget),
    SizingEnded,
    /// Whether the radius tracks the model's size (ZBrush calls it Dynamic)
    /// rather than staying a fixed number of millimetres.
    DynamicRadiusToggled(bool),
    /// A key was pressed and nothing in the widget tree wanted it.
    ///
    /// **Carries the key, not the message the key means.** The subscription
    /// that raises it cannot decide that itself: `iced::event::listen_with`
    /// takes a bare `fn` pointer (`iced_futures-0.14.0/src/event.rs:26`), so
    /// its callback captures nothing and can never see whether a modal card is
    /// up over the document. Decoding therefore happens in `Brokkr::on_key`,
    /// which can — and that is the one place the modal guard has to live.
    ///
    /// A press that spells nothing is still sent, and dropped there. The
    /// alternative, filtering in the subscription, would mean two places that
    /// know which keys exist.
    KeyPressed {
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    },
    /// A left press that no widget in the tree wanted. **Nothing acts on it;
    /// it exists so that something arrives.**
    ///
    /// This is the application's only blur signal, and it is needed because
    /// `text_input` blurs itself without saying so: any left press that is not
    /// over its own bounds sets `state.is_focused = None`
    /// (`text_input.rs:723-735`) and does not capture, and no ancestor stops
    /// it -- `Row::update` forwards every event to every child regardless of
    /// capture (`row.rs:261-271`). The bodies list is a FIXED six or eight
    /// rows tall, so with a handful of bodies there is empty scrollable below
    /// the last row that belongs to no widget at all. A press there used to
    /// produce no message whatsoever, which left `Brokkr::renaming` set and
    /// the rename field DRAWN BUT DEAD: the user's next keystroke reached
    /// `key_event` as Ignored and fired a tool shortcut -- `s` starting a
    /// brush-radius drag, `x`/`y`/`z` flipping the mirror planes -- which is
    /// verbatim the class of bug `viewport::route_pointer`'s header records as
    /// fixed.
    ///
    /// Left button only, and that is not an oversight: `text_input` blurs on a
    /// LEFT press, so a right press leaves the field focused and must leave
    /// the rename alone. Presses that a widget did want never get here at all
    /// -- `key_event` drops captured events, and the field, a row's
    /// `mouse_area` and the viewport shader all capture their own.
    PressedNothing,
    /// Close the right-click menu.
    MenuClosed,
    /// Turn the model so the face the cube menu was opened on becomes this
    /// direction.
    OrientFace(brokkr_core::Facing),
    /// Answer the "this looks like it came in lying down" prompt. `false`
    /// leaves the model exactly as imported.
    OrientPromptAnswered(bool),
    /// Open a top bar menu, or close it if it is the one already open.
    TopMenuToggled(TopMenu),
    /// Put the diagnostics on the clipboard, for pasting into a bug report.
    DiagnosticsCopied,
    /// Open the issue tracker in a browser, with what we know prefilled.
    IssueOpened,
    /// Typing in a number's field, by the key `ui::number` was given. Held
    /// as a draft until it is submitted. See `ui.rs`.
    FieldEdited(&'static str, String),
    /// A number's slider moved or its field was submitted: the draft under
    /// the key goes, and then the message carried is acted on as if it had
    /// come alone. `Brokkr::update` unwraps it before anything else.
    FieldSet(&'static str, Box<Message>),
    /// A field's draft did not read as a number and is let go.
    FieldDropped(&'static str),
    /// Edit, Preferences: the card that holds the settings of the
    /// application rather than of the document. See `docs/WORKSPACES.md`.
    /// A button of the tool rail. See `app/rail.rs`.
    RailChosen(Rail),
    /// Open a workspace. See `app/workspace.rs`.
    WorkspaceChosen(Workspace),
    /// Back to the workspace before this one: Tab.
    WorkspaceBack,
    /// A tab of the dock.
    DockTabChosen(DockTab),
    /// A number key, 1 to 8 as 0 to 7: the open workspace's library's
    /// brush of that place.
    LibraryKey(usize),
    /// A palette tile was right clicked: offer the keys to put it on. By
    /// its place on the palette. See `app/keys.rs`.
    KeyMenuOpened(usize),
    /// Put the brush whose key row is up on this key, 0 for the key 1.
    KeyPut(usize),
    KeyMenuClosed,
    PreferencesOpened,
    PreferencesClosed,
    /// Help, Keyboard shortcuts: every key and held modifier, on a card.
    ShortcutsOpened,
    ShortcutsClosed,
    PressureToggled(bool),
    PressureCurveChanged(f32),
    TiltToggled(bool),
    ResetPressurePeak,
    Undo,
    Redo,
    /// Throw the model away and start from a fresh sphere.
    ResetSphere,
    /// Write the sculpt out. Opens a dialog first.
    Export(ExportFormat),
    /// Start a fresh sculpt.
    NewSculpt,
    /// Open a saved sculpt: ask for a file, then load whatever came back.
    OpenRequested,
    OpenChosen(Option<std::path::PathBuf>),
    /// Open a named file straight from the recent list, with no dialog.
    OpenRecent(std::path::PathBuf),
    /// Load the crash net left behind by a session that did not save.
    RecoverAutosave,
    /// Show the welcome screen, from the Help menu.
    WelcomeOpened,
    /// Dismiss it. Escape does this too.
    WelcomeClosed,
    /// The TinkerAtlas feed came back, or did not.
    ArticlesLoaded(Result<Vec<crate::articles::Article>, String>),
    /// Ask again after a failure.
    ArticlesRetried,
    /// The answer from the update check: `Some` when a build worth telling the
    /// user about is published. Never an error.
    ///
    /// **A different ordinal, not a different commit.** The old check compared
    /// git SHAs, which have no order, so it could only ever say *different*.
    /// This compares build ordinals from a signed manifest, so it can say which
    /// is newer -- and during a rollback the offer names a LOWER one, which is
    /// why the wording comes from [`crate::update::Offer::headline`] rather
    /// than being written at the call site.
    ///
    /// A failure is silence, deliberately: an update check that reports its own
    /// network problems is a check that interrupts people to tell them about
    /// GitHub. See `update::check`.
    UpdateChecked(Option<crate::update::Offer>),
    /// Accept a verified offer: download the payload and stop there.
    ///
    /// **Downloads, and does not install.** The bytes are verified against the
    /// signed manifest and left on disk; nothing is made executable and nothing
    /// restarts until [`Self::UpdateInstallPressed`].
    UpdateDownloadRequested,
    /// Take the offer to go back to the build that was replaced.
    ///
    /// Raised only when a crash report is pending, the running ordinal matches
    /// what this install put in place, and the kept copy is still there with a
    /// matching digest. Offered rather than performed: the application drew a
    /// frame, so the user is at a window and can decide. The automatic revert
    /// exists for the user who never got one.
    UpdateRevertRequested,
    /// The Install button. Resolves the window the way `WindowClose` does --
    /// the view has no window id to hand, and `iced::window::latest()` is how
    /// this application gets one.
    UpdateInstallPressed,
    /// Install the downloaded build and restart into it.
    ///
    /// **The update question is asked by the button, not by a modal; the WORK
    /// question goes through `guard`.** That split is the point. `guard` only
    /// prompts `if would_lose_work`, so routing the whole thing through it
    /// would restart the app under anyone whose document happened to be saved --
    /// but the consequence of pressing Install is exactly what `guard` is for,
    /// so `PendingAction::Restart` goes through it and nothing touches the disk
    /// until the work question is settled.
    UpdateInstallRequested(iced::window::Id),
    /// The swap finished, or said why not. Carries the window because only the
    /// interface thread may close it, and the swap does not run there.
    UpdateInstalled(iced::window::Id, Result<u64, String>),
    /// Where the verified payload landed, or why it did not.
    UpdateDownloaded(Result<std::path::PathBuf, String>),
    /// The update-check tick, from either surface that carries it.
    ///
    /// Two surfaces on purpose: the welcome screen, beside the articles tick it
    /// sits next to, and Help, because the default is `always` and a switch
    /// reachable only from a screen the user turned off is not a switch.
    UpdateCheckSet(bool),
    /// Open a TinkerAtlas link in the browser: an article, or one of the two
    /// buttons on the welcome screen. The link is checked against the one host
    /// it may lead to before anything is spawned -- see
    /// [`crate::articles::open_in_browser`].
    LinkOpened(String),
    /// Start the browser half of signing in to TinkerAtlas.
    SignInRequested,
    /// The sign-in came back, with an account or with why not.
    SignInFinished(Result<crate::account::Account, String>),
    /// The signed-in user's avatar arrived, or did not. `None` is ordinary:
    /// no picture, an unreachable host, a format we do not decode.
    AvatarLoaded(Option<iced::widget::image::Handle>),
    /// Forget the stored account. Local only: it does not revoke the token,
    /// which is done on the site under desktop connections.
    SignOutRequested,
    /// The "show this on startup" tick, which is written through immediately:
    /// a preference that only lands when the dialog is dismissed the right way
    /// is one that silently forgets.
    WelcomeOnStartupSet(bool),
    /// Import a mesh: ask for a file, then read and voxelise whatever came back.
    ImportRequested,
    ImportChosen(Option<std::path::PathBuf>),
    /// The finished import, or why it failed.
    ///
    /// The payload is carried in a shared slot because `Message` derives `Debug`
    /// and `Clone` while `Volume` derives neither, so it cannot be put in a
    /// variant directly. See [`ImportPayload`].
    ImportLoaded(ImportPayload),
    /// Save. Over the current file if there is one, otherwise as a new one.
    SaveRequested,
    SaveAsRequested,
    SaveChosen(Option<std::path::PathBuf>),
    /// Export, in two halves for the same reason: ask, then write.
    ExportRequested(ExportFormat),
    ExportChosen(ExportFormat, Option<std::path::PathBuf>),
    /// The fit test for the parting keys, in the same two halves: ask where
    /// it goes, then write it. See `app/fit_test.rs`.
    FitTestRequested,
    FitTestChosen(Option<std::path::PathBuf>),
    /// Rebuild the volume at a different voxel size, which is the explicit
    /// operation that increases or reduces detail.
    Resample(f32),
    /// Typing in the working-size field. Held as text so a half-typed number
    /// is not rounded or rejected mid-keystroke.
    WorkingSizeTyped(String),
    /// Commit the working-size field: scale the model so its longest dimension
    /// is that many millimetres. Free, and buys no detail -- see
    /// `Volume::rescale`.
    WorkingSizeCommitted,
    /// The title bar was pressed: start moving the window. The bar IS the
    /// title bar -- the window is undecorated -- so this is what replaces the
    /// one the compositor would otherwise have drawn.
    TitleBarDragged,
    /// Double-click on the title bar, which everywhere means maximise.
    TitleBarDoubleClicked,
    WindowMinimise,
    WindowClose,
    /// A press on one of the window's resize edges. Undecorated windows get no
    /// resize border from the compositor, so the application draws its own.
    ResizeStarted(iced::window::Direction),
    /// The window manager asked to close. Carries the window, because
    /// `exit_on_close_request(false)` means nothing closes it but us.
    CloseRequested(iced::window::Id),
    /// An answer to the unsaved-work prompt.
    ConfirmAnswered(ConfirmChoice),
    /// A path chosen by the Save button *inside* the prompt. Separate from
    /// `SaveChosen` because this one continues to the pending action after a
    /// successful write, and must not after a failed one.
    SavedThenContinue(Option<std::path::PathBuf>),
    SpaceMouse(SpaceMouseSetting),
    /// Open or close one block of the properties panel.
    SectionToggled(PanelSection),
    /// Show or hide the stats readout over the viewport. Collapsed it is one
    /// icon in the corner; open it is seven lines of numbers.
    StatsToggled,
    /// Hand every brick back to the mesh pool and mesh them again, from the
    /// button on the overflow banner.
    ///
    /// **User-pressed and never automatic**, which was the open question and is
    /// now the answer: it is a synchronous whole-document remesh, and spending
    /// seconds of one uninvited mid-gesture is a worse surprise than the banner
    /// it would silence. See [`crate::app::Brokkr::rebuild_view`].
    ViewRebuilt,
    /// Choose what a left drag does. Pressing the live tool goes back to
    /// [`crate::app::Tool::Sculpt`], so this covers arming AND disarming and
    /// the cut costs no second variant.
    ToolChanged(Tool),

    // --- the body panel ------------------------------------------------------
    /// Make this body the one edits land on.
    BodySelected(brokkr_core::NodeId),
    /// Flip one row's own eye.
    ///
    /// Its OWN eye, never the resolved answer: an ancestor folder's eye and solo
    /// are masks applied on top, and a click here must not silently rewrite a
    /// bit the user cannot see.
    BodyVisibilityToggled(brokkr_core::NodeId),
    /// Flip the eye of whichever body is active, for `ctrl+comma`.
    ///
    /// A variant of its own rather than `BodyVisibilityToggled(active)`, because
    /// `viewport::shortcut` is a **pure decode** of a key and cannot see the
    /// document -- that separation is what lets every shortcut be tested without
    /// a window, and it is worth one message to keep.
    ActiveBodyVisibilityToggled,
    /// Turn every eye in the document on, for `ctrl+alt+comma`. The way out of
    /// having hidden something and lost track of what.
    EveryBodyShown,
    /// Open or close the small menu the `+` button drops.
    PrimitiveMenuToggled,
    /// Add a primitive as a NEW body, placed clear of everything already there.
    PrimitiveAdded(brokkr_core::PrimitiveKind),
    /// Remove the active body. May raise a prompt first; see
    /// `Brokkr::pending_delete`.
    BodyDeleted,
    /// Go ahead with a delete the prompt asked about.
    BodyDeleteConfirmed,
    BodyDeleteCancelled,
    /// Start renaming a row: a DOUBLE click on it, which is Photoshop's
    /// gesture and not the plan's single click.
    ///
    /// Single click had to go: the name cell is `Length::Fill` and therefore
    /// the largest target in the row, so a single click there is how a body
    /// gets selected. Giving the name its own `mouse_area` does not rescue it
    /// either -- `MouseArea::update` captures a left press whenever
    /// `on_double_click` is set, double or not (`mouse_area.rs:394-398`), so an
    /// inner area would eat the press the row's own selection depends on.
    BodyRenameBegan(brokkr_core::NodeId),
    /// A keystroke in the rename field, already clamped to what the file
    /// format's fixed name field can hold.
    BodyRenameEdited(String),
    /// Enter in the rename field. See `Brokkr::commit_rename` for where the
    /// commit actually happens, which is NOT this arm.
    BodyRenameSubmitted,
    /// Copy the active body, in place, as a new row directly below it.
    ///
    /// No payload: the active row is the subject, exactly as `BodyDeleted`'s is.
    /// A `NodeId` here would be a second answer to "which body" beside
    /// `Document::active`, and the two would disagree the first time a verb
    /// button was pressed while a stale id sat in a queued message.
    BodyDuplicated,
    /// Merge the active body down into the body directly below it.
    ///
    /// No payload, for the reason `BodyDuplicated` gives. May raise a prompt
    /// first; see `Brokkr::pending_merge`.
    BodyMergedDown,
    /// Go ahead with a merge the prompt asked about.
    BodyMergeConfirmed,
    BodyMergeCancelled,
    /// Split the active body into its loose parts.
    ///
    /// No payload, for the reason `BodyDuplicated` gives. Always raises the
    /// preview card first, because the number of parts is the whole of what
    /// there is to decide about; see `Brokkr::pending_split`.
    BodySplit,
    /// Go ahead with the split the preview card described.
    BodySplitConfirmed,
    BodySplitCancelled,
    /// `ctrl+G`: wrap the active row in a new folder, in place, no dialog.
    BodyGrouped,
    /// `ctrl+shift+G`: dissolve the folder the active row sits in.
    ///
    /// The PARENT and not the row, because the active row is always a body and
    /// a body has nothing to dissolve. That is also what makes the pair
    /// symmetric: ctrl+G then ctrl+shift+G leaves the document as it was.
    BodyUngrouped,
    /// The pointer moved over row `over` while a row is being dragged,
    /// `fraction` of the way down that row's height.
    ///
    /// **A fraction and not a pixel offset**, because which of a row's bands
    /// the pointer is in is the whole of the gesture and a band is a share of
    /// the row rather than a number of pixels -- the list is 32 px a row with
    /// pictures and 22 without. `mouse_area::on_move` hands over a point in the
    /// row's own space and the panel divides by the height it laid the row out
    /// at, which is why that height is `Length::Fixed` and not left to the
    /// content.
    ///
    /// Raised only while a drag is armed: the panel attaches no `on_move` at
    /// all otherwise, so an idle pointer over the list costs nothing.
    BodyRowDragged {
        over: usize,
        fraction: f32,
    },
    /// Fold a folder's children away in the panel, or show them again.
    ///
    /// Names the folder rather than acting on the active row: the chevron is
    /// drawn on the folder it belongs to, and a folder is never the active row.
    FolderCollapseToggled(brokkr_core::NodeId),
    /// Delete a folder and everything in it.
    ///
    /// **Deliberately not `BodyDeleted` with a folder in it.** The verb row's
    /// Delete names `Document::active`, which always holds a field, and this
    /// names a folder -- so no state of the panel, collapsed least of all, can
    /// make one of them do the other's job. In ZBrush it can, and a user
    /// reported losing an unrecoverable hour to it.
    FolderDeleted(brokkr_core::NodeId),
    /// Show only this row's subtree, and leave every eye alone.
    ///
    /// A view MODE and never a document change: it is `Option<NodeId>` on
    /// `Brokkr`, it is passed to `resolve_visibility` as a parameter, and it is
    /// written nowhere. Entering it on a row that is hidden is the one exception
    /// — that turns the row's own eye and its ancestors' back on, because
    /// "show me only this" with nothing on screen is not a mode anyone asked
    /// for — and *that* part is an ordinary undoable edit.
    SoloEntered(brokkr_core::NodeId),
    /// Leave solo. Escape, `ctrl+alt+comma`, and the header indicator's own
    /// exit all send this.
    ///
    /// It restores nothing, because it changed nothing: every hand-set eye is
    /// exactly where the user left it. That is the whole reason solo is a mode
    /// rather than a saved-visibility vector — every shipped version of the
    /// vector design loses the hand-set set on the way out.
    SoloExited,
    /// Push every drawn body apart on screen, or bring them back. A view and
    /// never an edit; a press on a body while apart selects it and brings
    /// them back. See `app/explode.rs`.
    ExplodeToggled,
    /// Show only the active body, or everything again: solo on the active
    /// body, from the hierarchy's header. See `app/explode.rs`.
    SoloActiveToggled,
    /// Show or hide the thumbnail column. Session state, so it must NOT dirty
    /// the document: nothing about it is written to the file.
    ThumbnailsToggled,
    /// Show or hide the mask's tint on the model.
    ///
    /// **It governs the TINT and nothing else.** The standing mask card is
    /// unconditional, so switching this off cannot reach the state "a mask is
    /// active and nothing on screen says so". It is view state, so it must not
    /// dirty the document, and it changes no protection and marks no brick
    /// dirty: the polarity and the strength are uniforms the shader reads.
    ShowMaskToggled,
    /// How strongly protection is tinted, 0..1.
    ///
    /// A view strength and never a protection strength — see
    /// [`brokkr_gpu::Uniforms::mask_tint`].
    MaskTintChanged(f32),
    /// The held peek key went down, or came back up.
    ///
    /// Kept alongside the toggle rather than replaced by it: it is the faster
    /// gesture for a momentary look, and the way back for a user who switched
    /// the tint off and forgot. Two messages rather than one toggle because the
    /// key repeats while it is held, and a toggle would strobe.
    MaskPeekStarted,
    MaskPeekEnded,
    /// One of the studio brush controls. See `app/studio.rs`.
    Studio(StudioMessage),
    /// One of the form tools' controls. See `app/form.rs`.
    Form(FormMessage),
    /// Shell or extract. See `app/hollow.rs`.
    Hollow(HollowMessage),
    /// Twist, bend or taper. See `app/deform.rs`.
    Deform(DeformMessage),
    /// The revolve's axis up or away. See `app/revolve.rs`.
    Revolve(RevolveMessage),
    /// One of the controls of the brushes that add. See `app/adding.rs`.
    Add(AddMessage),
    /// A vector displacement map asked for, taken in, or the maps read
    /// again. See `app/maps.rs`.
    Maps(MapsMessage),
    /// The workbench: onto it, a fresh piece, or back. See
    /// `app/workbench.rs`.
    Workbench(WorkbenchMessage),
    /// The measure: begin one, or clear what was measured. View state: it
    /// changes no voxel and is not saved. See `app/measure.rs`.
    Measure(MeasureMessage),
    /// How many texels on a side a baked texture is: 1024, 2048 or 4096. A
    /// setting of the application, like the export detail.
    TextureSizeChanged(u32),
    /// Draw paint as its own colour with no light on it, or lit like the
    /// clay. View state: a wash is shading, and a painter has to be able to
    /// see it without the viewport's light on top.
    FlatPaintToggled,
    /// Draw painted slots in their filament colours, or as bare clay.
    ///
    /// View state on the mask toggle's terms: it changes no slot, dirties no
    /// brick and does not dirty the document. See
    /// [`brokkr_gpu::Uniforms::paint_shown`].
    ShowPaintToggled,
    /// Draw creases darker and edges lighter than the light alone draws them,
    /// or stop.
    ///
    /// View state like the two toggles above, with one difference the handler
    /// has to honour: the signal is carried by the mesh, so switching it ON
    /// remeshes every body with it and switching it OFF remeshes without. It
    /// changes no voxel and does not dirty the document.
    ShowCavityToggled,
    /// How finely the next export is meshed. A setting of the application and
    /// not of the document: it changes no voxel and is not saved with a
    /// sculpt.
    ExportDetailChanged(ExportDetail),
    /// Change the key the next parting uses. It changes no voxel.
    KeySettingChanged(KeySetting),
    /// The overhang view's switch and its one setting.
    Overhang(OverhangMessage),
    /// The colour brush's panel.
    Painting(PaintingMessage),
    /// The active body's layers of paint. See `app/layers.rs`.
    Layers(LayersMessage),
    /// The racks of paints. See `app/rack.rs`.
    Rack(RackMessage),
    Designer(DesignerMessage),
    Cores(CoresMessage),
    Ring(RingMessage),
    /// The airbrush. See `app/airbrush.rs`.
    Airbrush(AirbrushMessage),
    /// The one-click coats. See `app/coats.rs`.
    Coats(CoatsMessage),
    /// The Decal touch. See `app/decal.rs`.
    Decal(DecalMessage),
    /// How the paint brush touches the model: loaded, nearly dry, or a
    /// sponge. A setting of the brush, not of the document.
    PaintTouchChosen(PaintTouch),
    /// One of the settings of the drybrush or the sponge.
    PaintTouchChanged(TouchSetting),
    /// Put the camera back on the active body, keeping the angles.
    ///
    /// The recovery half of a free camera. A camera that can go anywhere can
    /// end up anywhere, and the answer is one keystroke back rather than a
    /// fence around where it may go.
    CameraFramedOnActive,
    /// Throw the active body's mask away. The map is MOVED into the history
    /// entry, so this allocates nothing however large the mask was.
    MaskCleared,
    /// Flip which side of the mask is protected. One bool, no bricks, no bytes.
    MaskInverted,
    /// Protect the whole body: clear, then invert, as one change.
    MaskAllApplied,
    /// Which of the four absolute filters the amount slider drives.
    MaskFilterChosen(MaskFilter),
    /// The amount slider moved, to a fraction in `0..=1`.
    ///
    /// **Absolute, and re-applied from the mask as it stood when the drag
    /// began** -- never accumulated. ZBrush's BlurMask is "press repeatedly for
    /// progressively more blur" and that is its top masking complaint; Maxon's
    /// own later fix is documented as "absolute rather than accumulative".
    MaskAmountChanged(f32),
    /// The amount slider was let go, which commits the whole drag as ONE entry.
    MaskAmountReleased,
    /// Build a whole mask out of the active body's geometry.
    ///
    /// The parameters are NOT carried here: they are the two sliders beside the
    /// buttons, so a press means "with what is on screen" and the message stays
    /// the same shape however many recipes there turn out to be.
    MaskGenerated(MaskGenerator),
    /// The feature-size slider moved, in MILLIMETRES.
    ///
    /// Millimetres and never voxels, which is where this beats ZBrush outright
    /// rather than matching it: its cavity masking is resolution-relative, so
    /// the same model at a different subdivision masks differently. A body here
    /// has one fixed voxel size, so "narrower than 1.5 mm" is stable and means
    /// something to somebody choosing a nozzle.
    MaskFeatureChanged(f32),
    /// The thickness slider moved, in VOXELS.
    ///
    /// Voxels and not millimetres, and that is the opposite choice from the one
    /// above for a real reason: the ceiling is
    /// [`brokkr_core::MAX_THICKNESS_VOXELS`], which is a property of the narrow
    /// band and not of the model, so a millimetre slider would have a maximum
    /// that moved every time the body was resampled. The millimetres are shown
    /// beside the number.
    MaskThicknessChanged(f32),
    /// Split the active body in two along its mask.
    ///
    /// No preview card, unlike [`Message::BodySplit`]: a masked split always
    /// makes exactly two bodies, so there is nothing a card could tell the user
    /// that they do not already see in the tint.
    BodySplitMasked,
}

/// Which mask a generator button asked for.
///
/// Three and not four: the half-space is a DRAG and not a button, because it
/// needs a plane and the cut tool already has the gesture that makes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskGenerator {
    /// Concave detail narrower than the feature slider.
    Cavity,
    /// Everything flat at that scale: the same pass read the other way.
    Smoothness,
    /// Solid material thinner than the thickness slider.
    Thickness,
}

impl MaskGenerator {
    pub const ALL: [MaskGenerator; 3] =
        [MaskGenerator::Cavity, MaskGenerator::Smoothness, MaskGenerator::Thickness];

    pub fn label(self) -> &'static str {
        match self {
            MaskGenerator::Cavity => "Cavity",
            MaskGenerator::Smoothness => "Smooth",
            MaskGenerator::Thickness => "Thin",
        }
    }
}
