// SPDX-License-Identifier: AGPL-3.0-only

//! Racks: sets of paints the user keeps, one of them in hand. Stage 2 of
//! `docs/PAINT-RACK.md`, sections 2 and 3; the index they are filled from
//! is [`crate::paint_index`].
//!
//! The rack in hand fills Paint's library, comes up at the pointer on P,
//! and its first eight are on the keys 1 to 8 while painting in colour.
//! Choosing a paint takes its colour into the brush, and its kind suits
//! the active layer: a wash, a stain or a contrast wants a layer of that
//! blend, so an empty active layer takes the blend and one with paint on
//! it gets a new layer over it, named for the paint. See `app/layers.rs`.
//!
//! Each rack is a file, `racks/<name>.rack` in the data directory, one
//! `paint = kind<TAB>hex<TAB>brand<TAB>name` a line, so that a rack can be
//! handed to someone. Which rack is in hand is a setting, `rack.conf`.

use std::path::{Path, PathBuf};

use iced::widget::{button, column, container, image, pick_list, row, text, text_input};
use iced::{Alignment, Element, Length};

use super::Brokkr;
use crate::icon;
use crate::message::{DesignerMessage, Message, PaintTouch, RackMessage, SearchBy};
use crate::paint_index::{self, Filter, Kind};
use crate::theme;

/// The directory of racks in the data directory.
const DIRECTORY: &str = "racks";
/// The setting that says which rack is in hand.
const IN_HAND: &str = "rack.conf";
/// How many results a search shows.
const RESULTS: usize = 40;
/// Swatches a line, on the panel and at the pointer.
const A_ROW: usize = 8;
/// How big a paint's picture is on the rack, in pixels.
const PICTURE: usize = 22;

/// One paint on a rack: from the index, or a colour of the user's own,
/// whose brand is empty.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RackPaint {
    pub name: String,
    pub brand: String,
    pub rgb: [u8; 3],
    pub kind: Kind,
}

impl RackPaint {
    fn of(at: usize) -> Option<Self> {
        let paint = paint_index::index().get(at)?;
        Some(Self {
            name: paint.name.to_string(),
            brand: paint.brand.to_string(),
            rgb: paint.rgb,
            kind: paint.kind,
        })
    }

    /// What it is called on a tooltip: its name, its brand and its kind.
    fn title(&self) -> String {
        if self.brand.is_empty() {
            format!("{} ({})", self.name, self.kind.label())
        } else {
            format!("{}, {} ({})", self.name, self.brand.replace('_', " "), self.kind.label())
        }
    }
}

/// A set of paints with a name.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Rack {
    pub name: String,
    pub paints: Vec<RackPaint>,
}

impl Rack {
    /// The rack as its file holds it.
    pub(crate) fn to_text(&self) -> String {
        let mut text = String::from("# A rack of paints: kind, colour, brand, name.\n");
        text.push_str(&format!("name = {}\n", self.name));
        for paint in &self.paints {
            text.push_str(&format!(
                "paint = {}\t{:02X}{:02X}{:02X}\t{}\t{}\n",
                paint.kind.code(),
                paint.rgb[0],
                paint.rgb[1],
                paint.rgb[2],
                paint.brand,
                paint.name,
            ));
        }
        text
    }

    /// A rack from what its file holds. A paint line it cannot read is
    /// left out; a file with no name is no rack.
    pub(crate) fn from_text(text: &str) -> Option<Self> {
        let mut rack = Rack::default();
        for (key, value) in crate::paths::entries(text) {
            match key {
                "name" => rack.name = value.to_string(),
                "paint" => {
                    let mut fields = value.split('\t');
                    let (Some(kind), Some(rgb), Some(brand), Some(name)) =
                        (fields.next(), fields.next(), fields.next(), fields.next())
                    else {
                        continue;
                    };
                    let (Some(kind), Some(rgb)) = (Kind::from_code(kind), paint_index::hex(rgb))
                    else {
                        continue;
                    };
                    let name = name.trim();
                    if name.is_empty() {
                        continue;
                    }
                    rack.paints.push(RackPaint {
                        name: name.to_string(),
                        brand: brand.trim().to_string(),
                        rgb,
                        kind,
                    });
                }
                _ => {}
            }
        }
        (!rack.name.is_empty()).then_some(rack)
    }

    /// The rack a first run starts with: a dozen of Citadel's, a wash and
    /// a contrast among them.
    fn starter() -> Self {
        const PAINTS: [(&str, &str); 12] = [
            ("Base", "Abaddon Black"),
            ("Layer", "White Scar"),
            ("Base", "Mephiston Red"),
            ("Base", "Macragge Blue"),
            ("Base", "Averland Sunset"),
            ("Base", "Caliban Green"),
            ("Base", "Mournfang Brown"),
            ("Base", "Rakarth Flesh"),
            ("Base", "Leadbelcher"),
            ("Base", "Retributor Armour"),
            ("Shade", "Nuln Oil"),
            ("Contrast", "Blood Angels Red"),
        ];
        let index = paint_index::index();
        let paints = PAINTS
            .iter()
            .filter_map(|(line, name)| {
                let at = index.iter().position(|paint| {
                    paint.brand == "Citadel_Colour" && paint.line == *line && paint.name == *name
                })?;
                RackPaint::of(at)
            })
            .collect();
        Rack { name: "Starter".to_string(), paints }
    }
}

/// The racks, and the rack panel's own state.
#[derive(Debug)]
pub(crate) struct RackState {
    pub racks: Vec<Rack>,
    pub in_hand: usize,
    /// Which paint of the rack in hand was chosen last.
    pub chosen: Option<usize>,
    /// Whether the index search is open.
    pub searching: bool,
    pub query: String,
    pub by: SearchBy,
    pub kind: Option<Kind>,
    pub brand: Option<&'static str>,
}

impl Default for RackState {
    fn default() -> Self {
        let racks =
            if cfg!(test) { Vec::new() } else { directory().map(load_all).unwrap_or_default() };
        let mut state = Self::with(racks);
        if !cfg!(test) {
            let wanted = crate::paths::config_file(IN_HAND)
                .and_then(|path| std::fs::read_to_string(path).ok())
                .and_then(|text| {
                    crate::paths::entries(&text)
                        .find(|(key, _)| *key == "in_hand")
                        .map(|(_, value)| value.to_string())
                });
            if let Some(at) =
                wanted.and_then(|name| state.racks.iter().position(|rack| rack.name == name))
            {
                state.in_hand = at;
            }
        }
        state
    }
}

impl RackState {
    /// These racks, or the starter if there are none.
    fn with(mut racks: Vec<Rack>) -> Self {
        if racks.is_empty() {
            racks.push(Rack::starter());
        }
        Self {
            racks,
            in_hand: 0,
            chosen: None,
            searching: false,
            query: String::new(),
            by: SearchBy::Name,
            kind: None,
            brand: None,
        }
    }

    pub(crate) fn rack(&self) -> &Rack {
        &self.racks[self.in_hand.min(self.racks.len() - 1)]
    }

    fn rack_mut(&mut self) -> &mut Rack {
        let at = self.in_hand.min(self.racks.len() - 1);
        &mut self.racks[at]
    }

    /// Put a paint on the rack in hand, in place of the one at `instead`
    /// or at the end, and keep the racks. Where it went, and whether the
    /// racks were kept.
    pub(super) fn keep_paint(
        &mut self,
        instead: Option<usize>,
        paint: RackPaint,
    ) -> (usize, Result<(), String>) {
        let paints = &mut self.rack_mut().paints;
        let at = match instead {
            Some(at) if at < paints.len() => {
                paints[at] = paint;
                at
            }
            _ => {
                paints.push(paint);
                paints.len() - 1
            }
        };
        self.chosen = Some(at);
        (at, self.save())
    }

    /// A name no rack has, from `wanted`.
    fn free_name(&self, wanted: &str) -> String {
        let taken = |name: &str| self.racks.iter().any(|rack| rack.name == name);
        if !taken(wanted) {
            return wanted.to_string();
        }
        (2..).map(|n| format!("{wanted} {n}")).find(|name| !taken(name)).expect("a free name")
    }

    /// Keep every rack, and which is in hand. Not from a test, which must
    /// not write the user's files.
    fn save(&self) -> Result<(), String> {
        if cfg!(test) {
            return Ok(());
        }
        let directory = directory().ok_or("no data directory")?;
        save_all(&directory, &self.racks)?;
        let path = crate::paths::config_file(IN_HAND).ok_or("no settings directory")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
        }
        std::fs::write(path, format!("in_hand = {}\n", self.rack().name))
            .map_err(|why| why.to_string())
    }
}

fn directory() -> Option<PathBuf> {
    crate::paths::data_file(DIRECTORY)
}

/// A file name for a rack: its name with what a file system will not
/// take turned to `_`.
fn file_name(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' })
        .collect();
    format!("{}.rack", stem.trim())
}

/// Every rack in `directory`, sorted by name.
fn load_all(directory: PathBuf) -> Vec<Rack> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut racks: Vec<Rack> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rack"))
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| Rack::from_text(&text))
        .collect();
    racks.sort_by_key(|rack| rack.name.to_lowercase());
    racks.dedup_by(|a, b| a.name == b.name);
    racks
}

/// Write every rack into `directory`, and take away the rack files that
/// are not one of them: a rack renamed or deleted.
fn save_all(directory: &Path, racks: &[Rack]) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|why| why.to_string())?;
    let kept: Vec<String> = racks.iter().map(|rack| file_name(&rack.name)).collect();
    for (rack, name) in racks.iter().zip(&kept) {
        std::fs::write(directory.join(name), rack.to_text()).map_err(|why| why.to_string())?;
    }
    if let Ok(entries) = std::fs::read_dir(directory) {
        for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
            let stale = path.extension().is_some_and(|extension| extension == "rack")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| !kept.iter().any(|kept| kept == name));
            if stale {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    Ok(())
}

/// A rack in the list to swap from: its place and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RackChoice(usize, String);

impl std::fmt::Display for RackChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}

/// A kind to narrow a search to, or any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KindChoice(Option<Kind>);

impl std::fmt::Display for KindChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.map_or("Any kind", Kind::label))
    }
}

/// A brand to narrow a search to, or any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BrandChoice(Option<&'static str>);

impl std::fmt::Display for BrandChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(brand) => f.write_str(&brand.replace('_', " ")),
            None => f.write_str("Any brand"),
        }
    }
}

fn swatch(rgb: [u8; 3]) -> iced::Color {
    iced::Color::from_rgb8(rgb[0], rgb[1], rgb[2])
}

impl Brokkr {
    pub(super) fn rack_changed(&mut self, message: RackMessage) {
        let state = &mut self.painting.rack;
        let mut keep = true;
        match message {
            RackMessage::Chosen(at) => {
                if at < state.racks.len() {
                    state.in_hand = at;
                    state.chosen = None;
                }
            }
            RackMessage::New => {
                let name = state.free_name("Rack");
                state.racks.push(Rack { name: name.clone(), paints: Vec::new() });
                state.in_hand = state.racks.len() - 1;
                state.chosen = None;
                state.searching = true;
                self.status = format!("made the rack {name}");
            }
            RackMessage::Duplicated => {
                let mut copy = state.rack().clone();
                copy.name = state.free_name(&format!("{} copy", copy.name));
                state.racks.push(copy);
                state.in_hand = state.racks.len() - 1;
            }
            RackMessage::Deleted => {
                let gone = state.rack().name.clone();
                state.racks.remove(state.in_hand.min(state.racks.len() - 1));
                if state.racks.is_empty() {
                    state.racks.push(Rack { name: "Rack".to_string(), paints: Vec::new() });
                }
                state.in_hand = state.in_hand.min(state.racks.len() - 1);
                state.chosen = None;
                self.status = format!("deleted the rack {gone}");
            }
            RackMessage::Renamed(name) => {
                // Not to a name another rack has: two racks would be one
                // file.
                let taken = state
                    .racks
                    .iter()
                    .enumerate()
                    .any(|(at, rack)| at != state.in_hand && rack.name == name);
                if taken || name.trim().is_empty() {
                    keep = false;
                }
                state.rack_mut().name = name;
            }
            RackMessage::Picked(at) => {
                let Some(paint) = state.rack().paints.get(at).cloned() else {
                    return;
                };
                state.chosen = Some(at);
                self.take_up_paint(&paint);
                keep = false;
            }
            RackMessage::Raised(at) => {
                let paints = &mut state.rack_mut().paints;
                if at > 0 && at < paints.len() {
                    paints.swap(at, at - 1);
                    state.chosen = Some(at - 1);
                }
            }
            RackMessage::Lowered(at) => {
                let paints = &mut state.rack_mut().paints;
                if at + 1 < paints.len() {
                    paints.swap(at, at + 1);
                    state.chosen = Some(at + 1);
                }
            }
            RackMessage::Removed(at) => {
                let paints = &mut state.rack_mut().paints;
                if at < paints.len() {
                    let gone = paints.remove(at);
                    state.chosen = None;
                    self.status = format!("took {} off the rack", gone.name);
                }
            }
            RackMessage::Searching => {
                state.searching = !state.searching;
                keep = false;
            }
            RackMessage::Query(query) => {
                state.query = query;
                keep = false;
            }
            RackMessage::By(by) => {
                state.by = by;
                keep = false;
            }
            RackMessage::KindFilter(kind) => {
                state.kind = kind;
                keep = false;
            }
            RackMessage::BrandFilter(brand) => {
                state.brand = brand;
                keep = false;
            }
            RackMessage::Tried(at) => {
                let Some(paint) = RackPaint::of(at) else { return };
                state.chosen = None;
                self.take_up_paint(&paint);
                keep = false;
            }
            RackMessage::Added(at) => {
                let Some(paint) = RackPaint::of(at) else { return };
                let rack = state.rack_mut();
                if rack.paints.contains(&paint) {
                    self.status = format!("{} is on the rack already", paint.name);
                    return;
                }
                self.status = format!("put {} on the rack", paint.name);
                rack.paints.push(paint);
            }
        }
        if keep && let Err(why) = self.painting.rack.save() {
            self.status = format!("the racks were not saved: {why}");
        }
    }

    /// Take a paint into the brush: its colour, and its kind's blend on
    /// the active layer. Puts the palette at the pointer away.
    fn take_up_paint(&mut self, paint: &RackPaint) {
        self.painting.colour = paint.rgb;
        self.studio.popup = None;
        self.status = format!("paint: {}", paint.name);
        // The way this kind of paint is put on, already chosen: a wash as a
        // wash, a contrast as a contrast, an air paint by the airbrush. Any
        // other touch is the user's to choose after, a contrast drybrushed
        // included. A plain paint keeps the touch in hand, unless that touch
        // was a wash's or a contrast's.
        if let Some(touch) = paint.kind.touch() {
            self.touch.kind = touch;
        } else if matches!(self.touch.kind, PaintTouch::Wash | PaintTouch::Contrast) {
            self.touch.kind = PaintTouch::Loaded;
        }
        self.suit_the_layer(&paint.name, paint.kind.blend(), paint.kind == Kind::Metallic);
    }

    /// The key from 1 to 8 in Paint, painting in colour: the rack's paint
    /// in that place.
    pub(super) fn rack_key(&mut self, key: usize) {
        if key < self.painting.rack.rack().paints.len() {
            self.rack_changed(RackMessage::Picked(key));
        }
    }

    /// The rack in hand's swatches, `across` a line, the chosen one lit.
    pub(super) fn rack_swatches(&self, across: usize) -> Element<'_, Message> {
        let state = &self.painting.rack;
        let paints = &state.rack().paints;
        let mut grid = column![].spacing(theme::S1);
        for (line_at, line) in paints.chunks(across).enumerate() {
            let mut swatches = row![].spacing(theme::S1);
            for (offset, paint) in line.iter().enumerate() {
                let at = line_at * across + offset;
                let lit = state.chosen == Some(at);
                let (style, _) = theme::tool_toggle(lit);
                let title = match at {
                    0..8 => format!("{}  {}", at + 1, paint.title()),
                    _ => paint.title(),
                };
                swatches = swatches.push(crate::ui::tip(
                    button(
                        image(super::paint_preview::handle(
                            paint.rgb,
                            paint.kind,
                            PICTURE,
                            self.painting.pooling,
                        ))
                        .width(Length::Fixed(PICTURE as f32))
                        .height(Length::Fixed(PICTURE as f32)),
                    )
                    .padding(1)
                    .style(style)
                    .on_press(Message::Rack(RackMessage::Picked(at))),
                    title,
                ));
            }
            grid = grid.push(swatches);
        }
        if paints.is_empty() {
            grid = grid.push(text("Empty").size(theme::LABEL_SIZE).color(theme::TEXT_MUTE));
        }
        grid.into()
    }

    /// The RACK block of Paint's library: which rack is in hand, its
    /// swatches, the chosen paint, and the index to fill it from.
    pub(super) fn rack_panel(&self) -> Element<'_, Message> {
        let state = &self.painting.rack;
        let choices: Vec<RackChoice> = state
            .racks
            .iter()
            .enumerate()
            .map(|(at, rack)| RackChoice(at, rack.name.clone()))
            .collect();
        let current = choices.get(state.in_hand).cloned();
        let icon_button = |name: icon::IconName, tip: &'static str, message: Message| {
            crate::ui::tip(
                button(icon::icon(name, theme::ICON_INLINE, theme::TEXT))
                    .padding(theme::PAD_CHOICE)
                    .style(theme::tool_button)
                    .on_press(message),
                tip,
            )
        };
        let header = row![
            pick_list(choices, current, |choice: RackChoice| Message::Rack(RackMessage::Chosen(
                choice.0
            )))
            .text_size(theme::LABEL_SIZE)
            .width(Length::Fill),
            icon_button(icon::IconName::Plus, "New rack", Message::Rack(RackMessage::New)),
            icon_button(
                icon::IconName::Copy,
                "Duplicate rack",
                Message::Rack(RackMessage::Duplicated)
            ),
            icon_button(icon::IconName::Trash, "Delete rack", Message::Rack(RackMessage::Deleted)),
        ]
        .spacing(theme::S1)
        .align_y(Alignment::Center);
        let name = text_input("Rack name", &state.rack().name)
            .on_input(|name| Message::Rack(RackMessage::Renamed(name)))
            .size(theme::LABEL_SIZE)
            .padding(theme::PAD_FIELD);

        let mut block = column![header, name, self.rack_swatches(A_ROW)].spacing(theme::S2);

        // The chosen paint: what it is, and its place on the rack.
        if let Some(at) = state.chosen
            && let Some(paint) = state.rack().paints.get(at)
        {
            let len = state.rack().paints.len();
            let word = |label: &'static str, message: Option<Message>| {
                button(text(label).size(theme::LABEL_SIZE))
                    .padding(theme::PAD_CHOICE)
                    .style(theme::tool_button)
                    .on_press_maybe(message)
            };
            block = block.push(
                row![
                    image(super::paint_preview::handle(
                        paint.rgb,
                        paint.kind,
                        PICTURE,
                        self.painting.pooling
                    ))
                    .width(Length::Fixed(PICTURE as f32))
                    .height(Length::Fixed(PICTURE as f32)),
                    text(paint.title())
                        .size(theme::LABEL_SIZE)
                        .color(theme::TEXT_DIM)
                        .width(Length::Fill),
                    word("Up", (at > 0).then(|| Message::Rack(RackMessage::Raised(at)))),
                    word("Down", (at + 1 < len).then(|| Message::Rack(RackMessage::Lowered(at)))),
                    word("Edit", Some(Message::Designer(DesignerMessage::Edit(at)))),
                    icon_button(
                        icon::IconName::Trash,
                        "Take off the rack",
                        Message::Rack(RackMessage::Removed(at))
                    ),
                ]
                .spacing(theme::S1)
                .align_y(Alignment::Center),
            );
        }

        block = block.push(
            row![
                crate::ui::toggle(
                    "Add paints",
                    state.searching,
                    Message::Rack(RackMessage::Searching),
                ),
                button(text("New paint").size(theme::LABEL_SIZE))
                    .padding(theme::PAD_CHOICE)
                    .style(theme::tool_button)
                    .on_press(Message::Designer(DesignerMessage::New)),
            ]
            .spacing(theme::S2)
            .align_y(Alignment::Center),
        );
        if state.searching {
            block = block.push(self.index_search());
        }

        block.into()
    }

    /// The index search: words or the colour in hand, a kind and a brand,
    /// and what they find.
    fn index_search(&self) -> Element<'_, Message> {
        let state = &self.painting.rack;
        let filter = Filter { kind: state.kind, brand: state.brand };
        let found: Vec<(usize, Option<f32>)> = match state.by {
            SearchBy::Name => paint_index::search(&state.query, filter, RESULTS)
                .into_iter()
                .map(|at| (at, None))
                .collect(),
            SearchBy::Colour => paint_index::nearest(self.painting.colour, filter, RESULTS)
                .into_iter()
                .map(|(at, distance)| (at, Some(distance)))
                .collect(),
        };
        let by = crate::ui::segmented(
            [(SearchBy::Name, "By name"), (SearchBy::Colour, "Nearest colour")],
            state.by,
            |by| Message::Rack(RackMessage::By(by)),
        );
        let kinds: Vec<KindChoice> = std::iter::once(KindChoice(None))
            .chain(Kind::ALL.map(|kind| KindChoice(Some(kind))))
            .collect();
        let brands: Vec<BrandChoice> = std::iter::once(BrandChoice(None))
            .chain(paint_index::brands().iter().map(|brand| BrandChoice(Some(brand))))
            .collect();
        let filters = row![
            pick_list(kinds, Some(KindChoice(state.kind)), |choice: KindChoice| {
                Message::Rack(RackMessage::KindFilter(choice.0))
            })
            .text_size(theme::LABEL_SIZE)
            .width(Length::Fill),
            pick_list(brands, Some(BrandChoice(state.brand)), |choice: BrandChoice| {
                Message::Rack(RackMessage::BrandFilter(choice.0))
            })
            .text_size(theme::LABEL_SIZE)
            .width(Length::Fill),
        ]
        .spacing(theme::S1);

        let mut list = column![].spacing(2);
        for (at, distance) in found {
            let paint = &paint_index::index()[at];
            let mut label = format!("{}  {}", paint.name, paint.maker());
            if let Some(distance) = distance {
                label.push_str(&format!("  {:.1}", distance * 100.0));
            }
            list = list.push(
                row![
                    container(text(""))
                        .width(14)
                        .height(14)
                        .style(theme::swatch(swatch(paint.rgb))),
                    button(text(label).size(theme::LABEL_SIZE))
                        .width(Length::Fill)
                        .padding(theme::PAD_CHOICE)
                        .style(theme::section_heading)
                        .on_press(Message::Rack(RackMessage::Tried(at))),
                    text(paint.kind.label()).size(theme::LABEL_SIZE).color(theme::TEXT_MUTE),
                    crate::ui::tip(
                        button(icon::icon(icon::IconName::Plus, theme::ICON_INLINE, theme::TEXT))
                            .padding(theme::PAD_CHOICE)
                            .style(theme::tool_button)
                            .on_press(Message::Rack(RackMessage::Added(at))),
                        "Put on the rack",
                    ),
                ]
                .spacing(theme::S1)
                .align_y(Alignment::Center),
            );
        }
        let mut search = column![by].spacing(theme::S2);
        if state.by == SearchBy::Name {
            search = search.push(
                text_input("Search paints", &state.query)
                    .on_input(|query| Message::Rack(RackMessage::Query(query)))
                    .size(theme::LABEL_SIZE)
                    .padding(theme::PAD_FIELD),
            );
        }
        search.push(filters).push(list).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn rack(app: &mut Brokkr, message: RackMessage) {
        update(app, Message::Rack(message));
    }

    fn app() -> Brokkr {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(brokkr_core::BrushKind::Paint));
        update(&mut app, Message::WorkspaceChosen(crate::message::Workspace::Paint));
        app
    }

    fn at_in_index(brand: &str, line: &str, name: &str) -> usize {
        paint_index::index()
            .iter()
            .position(|paint| paint.brand == brand && paint.line == line && paint.name == name)
            .expect("in the index")
    }

    /// The starter rack is whole: every one of its paints is in the index.
    #[test]
    fn the_starter_rack_is_whole() {
        assert_eq!(Rack::starter().paints.len(), 12);
    }

    proptest! {
        /// Any rack written to its file reads back as it was.
        #[test]
        fn a_rack_reads_back_from_its_file(
            name in "[A-Za-z][A-Za-z0-9 ]{0,20}",
            paints in proptest::collection::vec(
                ("[A-Za-z][A-Za-z0-9 '!]{0,20}", 0usize..11, any::<[u8; 3]>(), proptest::bool::ANY),
                0..20,
            ),
        ) {
            let rack = Rack {
                name: name.trim().to_string(),
                paints: paints
                    .into_iter()
                    .map(|(name, kind, rgb, custom)| RackPaint {
                        name: name.trim().to_string(),
                        brand: if custom { String::new() } else { "Vallejo".to_string() },
                        rgb,
                        kind: Kind::ALL[kind],
                    })
                    .collect(),
            };
            prop_assume!(!rack.name.is_empty());
            prop_assert_eq!(Rack::from_text(&rack.to_text()), Some(rack));
        }
    }

    /// Racks kept in a directory come back, a renamed one is one file and
    /// not two, and a deleted one is gone.
    #[test]
    fn racks_are_kept_as_files() {
        let directory = std::env::temp_dir().join(format!("brokkr-racks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let mut racks =
            vec![Rack::starter(), Rack { name: "Skin tones".to_string(), paints: vec![] }];
        save_all(&directory, &racks).expect("written");
        assert_eq!(load_all(directory.clone()).len(), 2);
        racks[1].name = "Skin".to_string();
        save_all(&directory, &racks).expect("written");
        let back = load_all(directory.clone());
        assert_eq!(
            back.iter().map(|rack| rack.name.as_str()).collect::<Vec<_>>(),
            ["Skin", "Starter"]
        );
        racks.remove(0);
        save_all(&directory, &racks).expect("written");
        assert_eq!(load_all(directory.clone()).len(), 1);
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A paint chosen from the rack is the colour in hand, on its key too;
    /// a wash over a painted layer makes a wash layer named for it, and
    /// over an empty layer makes that layer a wash.
    #[test]
    fn a_paint_chosen_takes_its_colour_and_its_kind_suits_the_layer() {
        let mut app = app();
        let starter = Rack::starter();
        update(&mut app, Message::LibraryKey(2));
        assert_eq!(
            app.painting.colour, starter.paints[2].rgb,
            "key 3 did not pick the third paint"
        );
        let blend = |app: &Brokkr| {
            let layers = app.doc.active_volume().layers();
            layers.metas()[layers.active()].blend
        };
        // An empty layer takes the wash's blend.
        let wash =
            starter.paints.iter().position(|paint| paint.kind == Kind::Wash).expect("a wash");
        rack(&mut app, RackMessage::Picked(wash));
        assert_eq!(blend(&app), brokkr_core::paint::layers::Blend::Wash);
        assert_eq!(app.doc.active_volume().layers().len(), 1);
        // A plain paint takes it back to normal, still empty.
        rack(&mut app, RackMessage::Picked(0));
        assert_eq!(blend(&app), brokkr_core::paint::layers::Blend::Normal);
        // Painted, a wash makes a layer of its own over it.
        update(&mut app, Message::Painting(crate::message::PaintingMessage::Coated));
        rack(&mut app, RackMessage::Picked(wash));
        let names: Vec<String> =
            app.doc.active_volume().layers().metas().into_iter().map(|meta| meta.name).collect();
        assert_eq!(names, ["Paint", "Nuln Oil"]);
        assert_eq!(blend(&app), brokkr_core::paint::layers::Blend::Wash);
        update(&mut app, Message::Undo);
        assert_eq!(app.doc.active_volume().layers().len(), 1, "the new layer was not one undo");
    }

    /// A paint of a kind with its own way of going on has that touch chosen
    /// for it, and the user may change it after: a contrast drybrushed
    /// stays on its contrast layer. A metallic goes on a metal layer.
    #[test]
    fn a_paint_kind_chooses_its_touch_and_the_user_may_change_it() {
        use brokkr_core::paint::layers::Blend;
        let mut app = app();
        let starter = Rack::starter();
        let at = |kind| {
            starter.paints.iter().position(|paint| paint.kind == kind).expect("in the starter")
        };
        let active = |app: &Brokkr| {
            let layers = app.doc.active_volume().layers();
            layers.metas()[layers.active()].clone()
        };
        rack(&mut app, RackMessage::Picked(at(Kind::Contrast)));
        assert_eq!(app.touch.kind, PaintTouch::Contrast);
        assert_eq!(active(&app).blend, Blend::Contrast);
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Drybrush));
        assert_eq!(app.touch.kind, PaintTouch::Drybrush);
        assert_eq!(
            active(&app).blend,
            Blend::Contrast,
            "a drybrush took the contrast off its layer"
        );
        rack(&mut app, RackMessage::Picked(at(Kind::Wash)));
        assert_eq!(app.touch.kind, PaintTouch::Wash);
        rack(&mut app, RackMessage::Picked(at(Kind::Paint)));
        assert_eq!(app.touch.kind, PaintTouch::Loaded, "a plain paint kept the wash's touch");
        rack(&mut app, RackMessage::Picked(at(Kind::Metallic)));
        assert!(active(&app).metal, "a metallic is not on a metal layer");
        assert_eq!(active(&app).blend, Blend::Normal);
    }

    /// Racks are made, filled from the index, ordered, emptied, swapped
    /// and deleted by their messages, and a colour of the user's own goes
    /// on by its name, from the paint designer.
    #[test]
    fn a_rack_is_made_filled_and_swapped() {
        let mut app = app();
        rack(&mut app, RackMessage::New);
        assert_eq!(app.painting.rack.rack().name, "Rack");
        assert!(app.painting.rack.searching);
        let black = at_in_index("Vallejo", "Model Color", "Black");
        rack(&mut app, RackMessage::Added(black));
        rack(&mut app, RackMessage::Added(black));
        assert_eq!(app.painting.rack.rack().paints.len(), 1, "a paint went on twice");
        update(&mut app, Message::Painting(crate::message::PaintingMessage::Colour([10, 200, 30])));
        let design = |app: &mut Brokkr, message| update(app, Message::Designer(message));
        design(&mut app, DesignerMessage::New);
        design(&mut app, DesignerMessage::Save);
        assert_eq!(app.painting.rack.rack().paints.len(), 1, "a colour with no name went on");
        design(&mut app, DesignerMessage::Name("Goblin".to_string()));
        design(&mut app, DesignerMessage::Save);
        let paints = &app.painting.rack.rack().paints;
        assert_eq!(paints[1].name, "Goblin");
        assert_eq!(paints[1].rgb, [10, 200, 30]);
        assert!(paints[1].brand.is_empty());
        rack(&mut app, RackMessage::Raised(1));
        assert_eq!(app.painting.rack.rack().paints[0].name, "Goblin");
        rack(&mut app, RackMessage::Removed(0));
        assert_eq!(app.painting.rack.rack().paints[0].name, "Black");
        // A name another rack has is not taken for a file.
        rack(&mut app, RackMessage::Renamed("Necrons".to_string()));
        rack(&mut app, RackMessage::Chosen(0));
        assert_eq!(app.painting.rack.rack().name, "Starter");
        rack(&mut app, RackMessage::Duplicated);
        assert_eq!(app.painting.rack.rack().name, "Starter copy");
        rack(&mut app, RackMessage::Deleted);
        assert_eq!(app.painting.rack.racks.len(), 2);
        // The last rack cannot go: an empty one takes its place.
        rack(&mut app, RackMessage::Deleted);
        rack(&mut app, RackMessage::Deleted);
        assert_eq!(app.painting.rack.racks.len(), 1);
        assert!(app.painting.rack.rack().paints.is_empty());
    }

    /// A paint tried from the search is the colour in hand and not on the
    /// rack; the panel draws with the search open either way.
    #[test]
    fn a_paint_is_tried_from_the_search() {
        let mut app = app();
        let khorne = at_in_index("Citadel_Colour", "Base", "Khorne Red");
        rack(&mut app, RackMessage::Tried(khorne));
        assert_eq!(app.painting.colour, paint_index::index()[khorne].rgb);
        assert_eq!(app.painting.rack.rack().paints.len(), 12);
        rack(&mut app, RackMessage::Searching);
        rack(&mut app, RackMessage::Query("khorne".to_string()));
        drop(app.rack_panel());
        rack(&mut app, RackMessage::By(SearchBy::Colour));
        rack(&mut app, RackMessage::KindFilter(Some(Kind::Wash)));
        drop(app.rack_panel());
    }
}
