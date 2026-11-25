// SPDX-License-Identifier: AGPL-3.0-only

//! Workspaces: Sculpt, Kitbash, Paint and Print, and moving between them
//! and back.
//!
//! `docs/WORKSPACES.md` is the design (stage U4), and the maintainer's
//! decisions of 2026-11-25 are its frame: four workspaces, Print one of
//! them, the keys 1 to 8 picking the first eight of the open workspace's
//! library.
//!
//! A workspace changes what the rail, the bar and the dock offer, and what
//! a left drag on the model does. It does not change the document: the
//! camera, the bodies, which is chosen, the mask, the mirror planes and the
//! one undo history are the same whichever is open, and a switch touches
//! none of them. Each workspace remembers its own tool, its dock's tab and
//! which of its sections are open, so Sculpt, then Paint, then Sculpt again
//! lands where it was left.

use brokkr_core::BrushKind;
use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Element, Length};

use super::{Brokkr, Tool};
use crate::message::{
    AddMessage, DockTab, MeasureMessage, Message, PaintTouch, PanelSection, Rail, StudioMessage,
    Workspace,
};
use crate::theme;

/// What a workspace keeps while another is open.
#[derive(Debug, Clone, Copy)]
pub(super) struct Memory {
    /// The rail's lit button when it was left: put back on the way in.
    pub rail: Option<Rail>,
    /// The thing to add that was in hand in Kitbash.
    pub insert: Option<super::Insert>,
    pub tab: DockTab,
    pub expanded: [bool; PanelSection::ALL.len()],
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            rail: None,
            insert: None,
            tab: DockTab::Library,
            expanded: PanelSection::ALL.map(PanelSection::open_by_default),
        }
    }
}

/// Where the window is: the open workspace, the one before it, and what
/// each remembers.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Desk {
    pub now: Workspace,
    pub last: Workspace,
    pub memory: [Memory; Workspace::ALL.len()],
}

impl Workspace {
    /// The rail's buttons in this mode, in order. Paint has no button
    /// for painting: painting is what Paint does when neither the mask nor
    /// a measurement is up.
    pub(super) fn rail(self) -> &'static [Rail] {
        match self {
            Workspace::Sculpt => &[Rail::Brush, Rail::Move, Rail::Mask, Rail::Cut, Rail::Measure],
            Workspace::Transform => &[Rail::Transform, Rail::Place, Rail::Measure],
            Workspace::Paint => &[Rail::Mask, Rail::Measure],
            Workspace::Print => &[Rail::Cut, Rail::Measure],
        }
    }

    /// Every tool the mode may have lit: its rail's, and Paint's painting,
    /// which has no button.
    pub(super) fn tools(self) -> &'static [Rail] {
        match self {
            Workspace::Paint => &[Rail::Paint, Rail::Mask, Rail::Measure],
            other => other.rail(),
        }
    }

    /// The tool a mode opens with the first time, and falls back to when
    /// what is lit is none of its own.
    fn first_tool(self) -> Rail {
        match self {
            Workspace::Sculpt => Rail::Brush,
            Workspace::Transform => Rail::Transform,
            Workspace::Paint => Rail::Paint,
            // Print is for looking, measuring and cutting, and a drag that
            // sculpted there would be a surprise: it opens measuring.
            Workspace::Print => Rail::Measure,
        }
    }

    /// Whether a key's message belongs to this mode. W is Transform's; C
    /// is Sculpt's and Print's; M is Sculpt's and Paint's; grouping bodies
    /// is Transform's. Anything else goes everywhere.
    pub(super) fn allows(self, message: &Message) -> bool {
        match message {
            Message::ToolChanged(Tool::Transform) => self == Workspace::Transform,
            Message::ToolChanged(Tool::Cut) => {
                matches!(self, Workspace::Sculpt | Workspace::Print)
            }
            Message::ToolChanged(Tool::Mask) => {
                matches!(self, Workspace::Sculpt | Workspace::Paint)
            }
            Message::BodyGrouped | Message::BodyUngrouped => self == Workspace::Transform,
            _ => true,
        }
    }

    /// Where the palette's tiles of this workspace's library begin, and
    /// how many there are: the keys 1 to 8 index into them. `None` for a
    /// workspace whose library is not the palette's.
    pub(super) fn palette_places(self) -> Option<std::ops::Range<usize>> {
        let named = |heading: &str| {
            super::presets::groups().find(|(group, _)| *group == heading).map(|(_, at)| at)
        };
        match self {
            Workspace::Sculpt => named("Sculpt"),
            Workspace::Transform => named("Add"),
            Workspace::Paint | Workspace::Print => None,
        }
    }
}

impl Brokkr {
    /// Open a workspace, keeping what the one being left had.
    pub(super) fn enter_workspace(&mut self, to: Workspace) -> iced::Task<Message> {
        let from = self.desk.now;
        if to == from {
            return iced::Task::none();
        }
        let rail = self.rail_lit();
        let expanded = self.expanded;
        let insert = self.inserts.what;
        let leaving = &mut self.desk.memory[from as usize];
        leaving.rail = Some(rail);
        leaving.expanded = expanded;
        if from == Workspace::Transform {
            leaving.insert = insert;
        }
        self.desk.last = from;
        self.desk.now = to;
        self.expanded = self.desk.memory[to as usize].expanded;
        let mut tasks = Vec::new();
        // What a left drag does belongs to the workspace: what was being
        // added is put away on the way out of Kitbash, and measuring on the
        // way out of any workspace, so neither is left armed under another
        // workspace's tools.
        if self.inserts.what.is_some() {
            tasks.push(self.update(Message::Add(AddMessage::Chosen(None))));
        }
        if self.measure.armed {
            tasks.push(self.update(Message::Measure(MeasureMessage::Armed(false))));
        }
        let tool = self.desk.memory[to as usize]
            .rail
            .filter(|rail| to.tools().contains(rail))
            .unwrap_or(to.first_tool());
        tasks.push(self.rail_chosen_in(tool));
        self.status = format!("{} workspace", to.label());
        iced::Task::batch(tasks)
    }

    /// Whether the cut, released, parts the body in two with keys rather
    /// than cutting material away: always in Print, never in Sculpt, and
    /// with Alt held anywhere else the cut can be had.
    pub(super) fn parting_cut(&self) -> bool {
        match self.desk.now {
            Workspace::Print => true,
            Workspace::Sculpt => false,
            _ => self.alt,
        }
    }

    /// After every message: what the open mode does with a drag stays the
    /// mode's.
    ///
    /// - A part taken up in Transform puts the gizmo down, so a drag sets
    ///   it down.
    /// - In Transform and in Print a drag never strokes the field: when a
    ///   tool of theirs puts itself down (a cut is one shot, the gizmo can
    ///   be refused) and the brush would be what is left, the mode's own
    ///   first tool is taken up, or measuring if that is refused.
    pub(super) fn keep_to_the_mode(&mut self) -> iced::Task<Message> {
        if self.keeping {
            return iced::Task::none();
        }
        self.keeping = true;
        let now = self.desk.now;
        let mut task = iced::Task::none();
        // A part taken up in Transform, from its palette or by a key, is
        // set down by a drag: the gizmo goes down so the drag reaches it.
        if now == Workspace::Transform && self.inserting() && self.tool != Tool::Sculpt {
            task = self.update(Message::ToolChanged(Tool::Sculpt));
        }
        if matches!(now, Workspace::Transform | Workspace::Print)
            && !now.tools().contains(&self.rail_lit())
        {
            task = iced::Task::batch([task, self.rail_chosen_in(now.first_tool())]);
            if !now.tools().contains(&self.rail_lit()) {
                task = iced::Task::batch([task, self.rail_chosen_in(Rail::Measure)]);
            }
        }
        self.keeping = false;
        task
    }

    /// A rail button, pressed by the workspace on the way in rather than
    /// by a hand: a mode already up is left up rather than put down.
    fn rail_chosen_in(&mut self, tool: Rail) -> iced::Task<Message> {
        if self.rail_lit() == tool {
            return iced::Task::none();
        }
        self.rail_chosen(tool)
    }

    /// The thing to add that Kitbash's Place button takes up: the one in
    /// hand when Kitbash was left, or a ball.
    pub(super) fn kitbash_insert(&self) -> super::Insert {
        self.desk.memory[Workspace::Transform as usize]
            .insert
            .unwrap_or(super::Insert::Shape(brokkr_core::solid::Form::Ball))
    }

    /// A key from 1 to 8: the open workspace's library's brush of that
    /// place, if it has one.
    pub(super) fn library_key(&mut self, index: usize) -> iced::Task<Message> {
        match self.desk.now {
            Workspace::Sculpt | Workspace::Transform => {
                let Some(places) = self.desk.now.palette_places() else {
                    return iced::Task::none();
                };
                if index >= places.len() {
                    return iced::Task::none();
                }
                self.update(Message::Studio(StudioMessage::PresetChosen(places.start + index)))
            }
            Workspace::Paint => match PaintTouch::ALL.get(index) {
                Some(touch) => self.update(Message::PaintTouchChosen(*touch)),
                None => iced::Task::none(),
            },
            Workspace::Print => iced::Task::none(),
        }
    }

    /// The switcher in the title bar.
    pub(super) fn workspace_switcher(&self) -> Element<'_, Message> {
        let choices = Workspace::ALL.into_iter().fold(row![].spacing(2), |assembled, which| {
            let on = which == self.desk.now;
            assembled.push(crate::ui::tip(
                button(text(which.label()).size(theme::LABEL_SIZE))
                    .padding(iced::Padding { top: 3.0, bottom: 3.0, left: 12.0, right: 12.0 })
                    .style(if on { theme::tool_button_active } else { theme::window_control })
                    .on_press(Message::WorkspaceChosen(which)),
                which.key(),
            ))
        });
        container(choices).padding(2).style(theme::timeline_track).into()
    }

    /// The dock down the right: the workspace's library and its inspector,
    /// on two tabs, and the bodies under both.
    pub(super) fn dock(&self) -> Element<'_, Message> {
        let now = self.desk.now;
        let tab = self.desk.memory[now as usize].tab;
        let tabs = [DockTab::Library, DockTab::Inspector].into_iter().fold(
            row![].spacing(theme::S1),
            |assembled, which| {
                let on = which == tab;
                assembled.push(
                    button(text(which.label(now)).size(theme::LABEL_SIZE).color(if on {
                        theme::TEXT
                    } else {
                        theme::TEXT_MUTE
                    }))
                    .padding(iced::Padding { top: 2.0, bottom: 4.0, left: 6.0, right: 6.0 })
                    .style(if on { theme::tool_button } else { theme::section_heading })
                    .on_press(Message::DockTabChosen(which)),
                )
            },
        );
        let body = match tab {
            DockTab::Library => self.library(now),
            DockTab::Inspector => self.inspector(now),
        };
        container(
            column![
                tabs,
                scrollable(
                    container(body)
                        .padding(iced::Padding { right: theme::S4, ..iced::Padding::ZERO })
                )
                .height(Length::Fill),
                // The bodies, always: which part a stroke, a coat, a move
                // or a cut goes to is chosen here in every mode, so it does
                // not fold away. See `docs/WORKSPACES.md`.
                self.bodies_block(),
            ]
            .spacing(theme::S3),
        )
        .padding(theme::PANEL_PADDING)
        .width(Length::Fixed(DOCK_WIDTH))
        .height(Length::Fill)
        .style(theme::panel)
        .into()
    }

    /// The bodies list at the foot of every mode's dock: a heading that
    /// does not fold, the SOLO badge beside it, and the list.
    fn bodies_block(&self) -> Element<'_, Message> {
        let mut heading = row![
            text(PanelSection::Bodies.title()).size(theme::HEADING_SIZE).color(theme::TEXT_MUTE)
        ]
        .spacing(theme::S2)
        .align_y(iced::Alignment::Center);
        if let Some(badge) = self.solo_badge(PanelSection::Bodies) {
            heading = heading.push(badge);
        }
        column![heading, self.bodies_panel()].spacing(theme::S2).into()
    }

    /// What a workspace offers to pick from.
    fn library(&self, now: Workspace) -> Element<'_, Message> {
        match now {
            Workspace::Sculpt => self.palette_panel(),
            Workspace::Transform => {
                column![self.palette_panel(), self.adding_library(),].spacing(theme::S3).into()
            }
            Workspace::Paint => self.painting_picker(true),
            Workspace::Print => self.export_panel(),
        }
    }

    /// The settings of what is in hand, in sections that fold.
    fn inspector(&self, now: Workspace) -> Element<'_, Message> {
        let sections: Vec<Element<'_, Message>> = match now {
            // A shape baked into the body is a Sculpt brush, and how it
            // meets the surface is its setting there.
            Workspace::Sculpt if self.inserting() => vec![
                self.section(PanelSection::Add, || self.adding_settings()),
                self.section(PanelSection::Detail, || self.detail_panel()),
            ],
            Workspace::Sculpt => vec![
                self.brush_section(),
                self.section(PanelSection::Stroke, || self.stroke_panel()),
                self.section(PanelSection::Pattern, || self.pattern_panel()),
                self.section(PanelSection::Form, || self.form_panel()),
                self.section(PanelSection::Workbench, || self.workbench_panel()),
                self.section(PanelSection::Detail, || self.detail_panel()),
            ],
            Workspace::Transform => vec![
                self.section(PanelSection::Add, || self.adding_settings()),
                self.section(PanelSection::Detail, || self.detail_panel()),
            ],
            Workspace::Paint => vec![
                self.brush_section(),
                self.section(PanelSection::Stroke, || self.stroke_panel()),
                self.section(PanelSection::Filament, || self.filament_panel()),
            ],
            Workspace::Print => vec![
                self.section(PanelSection::Detail, || self.detail_panel()),
                self.section(PanelSection::Filament, || self.filament_panel()),
            ],
        };
        column(sections).spacing(theme::S4).into()
    }

    /// The BRUSH section: the brush in hand's size, weight and edge, and
    /// for a brush on the field what it is made of.
    fn brush_section(&self) -> Element<'_, Message> {
        let in_hand = text(match self.tool {
            Tool::Mask => "Mask".to_string(),
            _ => self.live_brush_name(),
        })
        .size(theme::LABEL_SIZE)
        .color(theme::TEXT_DIM);
        let field_brush = self.tool == Tool::Sculpt
            && !matches!(self.brush.kind, BrushKind::Paint | BrushKind::Move);
        crate::ui::section(
            PanelSection::Brush.title(),
            self.expanded[PanelSection::Brush as usize],
            Message::SectionToggled(PanelSection::Brush),
            Some(in_hand.into()),
            move || {
                let mut block = column![self.radius_number(false)].spacing(theme::S3);
                if let Some(weight) = self.weight_number(false) {
                    block = block.push(weight);
                }
                block = block.push(
                    iced::widget::checkbox(self.dynamic_radius)
                        .label("scale with model")
                        .on_toggle(Message::DynamicRadiusToggled)
                        .text_size(theme::LABEL_SIZE),
                );
                // The edge drawn, as a cross-section from rim to rim: the
                // weight a stamp gives at each distance from its middle.
                // See `curve.rs` and issue 78.
                let profile = if field_brush {
                    self.studio.profile(self.brush.falloff)
                } else {
                    brokkr_core::brush::compound::Profile::of(self.brush.falloff)
                };
                let edge = crate::curve::curve(crate::curve::cross_section(move |distance| {
                    profile.weight(distance)
                }));
                if field_brush {
                    block = block.push(edge).push(self.profile_row()).push(self.studio_panel());
                } else if let Some(falloff) = self.falloff_choice() {
                    block = block.push(crate::ui::sub_label("Falloff")).push(edge).push(falloff);
                }
                block.into()
            },
        )
    }
}

/// How wide the dock is. The panel it replaced was 258; the inspector's
/// numbers and the five shapes of a profile were tight in that, seen on
/// the screen.
pub(super) const DOCK_WIDTH: f32 = 276.0;

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn app() -> Brokkr {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        app.welcome = false;
        app
    }

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    /// The parts of the document and the view that no switch may touch,
    /// written out so two can be compared.
    fn kept(app: &Brokkr) -> String {
        let volume = app.doc.active_volume();
        let samples: Vec<f32> = [
            glam::Vec3::ZERO,
            glam::Vec3::new(9.0, 0.0, 0.0),
            glam::Vec3::new(0.0, 9.9, 0.0),
            glam::Vec3::new(3.0, -4.0, 8.0),
        ]
        .iter()
        .map(|at| volume.sample_world(*at))
        .collect();
        format!(
            "{:?} {:?} {:?} {:?} {} {} {} {:?}",
            app.camera.target,
            (app.camera.distance, app.camera.yaw),
            app.symmetry,
            app.doc.active(),
            app.doc.node_count(),
            app.history.can_undo(),
            app.history.can_redo(),
            samples,
        )
    }

    fn workspace() -> impl Strategy<Value = Option<Workspace>> {
        prop_oneof![
            Just(Some(Workspace::Sculpt)),
            Just(Some(Workspace::Transform)),
            Just(Some(Workspace::Paint)),
            Just(Some(Workspace::Print)),
            // Tab: back to the one before.
            Just(None),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

        /// Any run of switches, by the switcher or by Tab, leaves the
        /// camera, the mirror planes, the bodies, the field and the undo
        /// history as they were.
        #[test]
        fn no_switch_changes_the_document(path in proptest::collection::vec(workspace(), 1..10)) {
            let mut app = app();
            // A mirror plane on first, so there is a setting to keep.
            update(&mut app, Message::SymmetryAxisToggled(brokkr_core::MirrorAxis::X));
            let before = kept(&app);
            for step in path {
                update(&mut app, match step {
                    Some(to) => Message::WorkspaceChosen(to),
                    None => Message::WorkspaceBack,
                });
                prop_assert_eq!(&kept(&app), &before);
                prop_assert!(app.desk.now.tools().contains(&app.rail_lit()),
                    "{:?} lit {:?}, which it does not have", app.desk.now, app.rail_lit());
            }
        }
    }

    /// Sculpt, then Paint, then Sculpt: the palette's brush in hand, the
    /// tab and an open section come back.
    #[test]
    fn a_workspace_is_found_as_it_was_left() {
        let mut app = app();
        update(&mut app, Message::Studio(StudioMessage::PresetChosen(4)));
        update(&mut app, Message::DockTabChosen(DockTab::Inspector));
        let form = app.expanded[PanelSection::Form as usize];
        update(&mut app, Message::SectionToggled(PanelSection::Form));
        update(&mut app, Message::WorkspaceChosen(Workspace::Paint));
        assert_eq!(app.rail_lit(), Rail::Paint);
        assert_eq!(app.brush.kind, BrushKind::Paint);
        assert_eq!(app.desk.memory[Workspace::Paint as usize].tab, DockTab::Library);
        update(&mut app, Message::WorkspaceBack);
        assert_eq!(app.desk.now, Workspace::Sculpt);
        assert_eq!(app.rail_lit(), Rail::Brush);
        let now = app.preset_now(String::new());
        let palette = app.studio.presets.palette();
        assert_eq!(Brokkr::lit(&palette, &now), Some(4), "the brush in hand did not come back");
        assert_eq!(app.desk.memory[Workspace::Sculpt as usize].tab, DockTab::Inspector);
        assert_eq!(app.expanded[PanelSection::Form as usize], !form);
    }

    /// Kitbash takes something to add up on the way in and puts it away
    /// on the way out, so a drag in Sculpt is a stroke again; going back
    /// takes the same thing up.
    #[test]
    fn transform_holds_its_part_and_only_there() {
        let mut app = app();
        assert!(!app.inserting());
        // Transform opens on the gizmo, and Place takes a part up as a body
        // of its own.
        update(&mut app, Message::WorkspaceChosen(Workspace::Transform));
        assert_eq!(app.rail_lit(), Rail::Transform);
        update(&mut app, Message::RailChosen(Rail::Place));
        assert!(app.inserting(), "Place took nothing up");
        assert!(app.inserts.separate, "a part in Transform was baked in");
        assert_eq!(app.rail_lit(), Rail::Place);
        let box_ = super::super::Insert::Shape(brokkr_core::solid::Form::Box);
        update(&mut app, Message::Add(AddMessage::Chosen(Some(box_))));
        update(&mut app, Message::WorkspaceChosen(Workspace::Sculpt));
        assert!(!app.inserting(), "Sculpt kept Transform's part in hand");
        update(&mut app, Message::WorkspaceChosen(Workspace::Transform));
        assert_eq!(app.inserts.what, Some(box_));
        // The gizmo puts the part down and Place takes the same one up.
        update(&mut app, Message::RailChosen(Rail::Transform));
        assert!(!app.inserting());
        update(&mut app, Message::RailChosen(Rail::Place));
        assert_eq!(app.inserts.what, Some(box_));
    }

    /// A shape chosen in Sculpt is baked into the body, and the same shape
    /// chosen in Transform is a body of its own.
    #[test]
    fn the_mode_says_baked_in_or_its_own_body() {
        let mut app = app();
        let ball = super::super::Insert::Shape(brokkr_core::solid::Form::Ball);
        update(&mut app, Message::Add(AddMessage::Chosen(Some(ball))));
        assert!(!app.inserts.separate, "a shape in Sculpt is not baked in");
        assert_eq!(app.rail_lit(), Rail::Brush);
        update(&mut app, Message::WorkspaceChosen(Workspace::Transform));
        update(&mut app, Message::Add(AddMessage::Chosen(Some(ball))));
        assert!(app.inserts.separate, "a part in Transform is baked in");
        assert_eq!(app.rail_lit(), Rail::Place, "a part taken up left the gizmo up");
    }

    /// The keys that belong to one mode do nothing in another, and a cut
    /// parts the body in Print and never in Sculpt.
    #[test]
    fn keys_and_the_cut_follow_the_mode() {
        let key = |character: &str| Message::KeyPressed {
            key: iced::keyboard::Key::Character(character.into()),
            modifiers: iced::keyboard::Modifiers::default(),
        };
        let mut app = app();
        update(&mut app, key("w"));
        assert_ne!(app.tool, Tool::Transform, "W raised the gizmo in Sculpt");
        update(&mut app, Message::WorkspaceChosen(Workspace::Transform));
        update(&mut app, key("m"));
        assert_ne!(app.tool, Tool::Mask, "M masked in Transform");
        update(&mut app, Message::WorkspaceChosen(Workspace::Paint));
        update(&mut app, key("c"));
        assert_ne!(app.tool, Tool::Cut, "C armed the cut in Paint");
        app.alt = true;
        update(&mut app, Message::WorkspaceChosen(Workspace::Sculpt));
        assert!(!app.parting_cut(), "Alt parts in Sculpt");
        update(&mut app, Message::WorkspaceChosen(Workspace::Print));
        app.alt = false;
        assert!(app.parting_cut(), "a cut in Print does not part");
        // And in Print a drag never strokes: what is lit is Print's.
        assert!(Workspace::Print.tools().contains(&app.rail_lit()));
    }

    /// The keys 1 to 8 pick from the open workspace's library: the
    /// palette's Sculpt group, its Add group, the paint's touches.
    #[test]
    fn the_number_keys_follow_the_workspace() {
        let mut app = app();
        update(&mut app, Message::LibraryKey(2));
        let now = app.preset_now(String::new());
        let palette = app.studio.presets.palette();
        assert_eq!(Brokkr::lit(&palette, &now), Some(2));
        update(&mut app, Message::WorkspaceChosen(Workspace::Paint));
        update(&mut app, Message::LibraryKey(2));
        assert_eq!(app.touch.kind, PaintTouch::ALL[2]);
        update(&mut app, Message::WorkspaceChosen(Workspace::Transform));
        update(&mut app, Message::LibraryKey(2));
        let places =
            Workspace::Transform.palette_places().expect("Kitbash's library is the palette's");
        let now = app.preset_now(String::new());
        let palette = app.studio.presets.palette();
        assert_eq!(Brokkr::lit(&palette, &now), Some(places.start + 2));
        // Print has no library of brushes: a key there does nothing.
        update(&mut app, Message::WorkspaceChosen(Workspace::Print));
        let before = kept(&app);
        update(&mut app, Message::LibraryKey(0));
        assert_eq!(kept(&app), before);
    }

    /// Tab goes back to the workspace before, and under a card does
    /// nothing.
    #[test]
    fn tab_goes_back_and_a_card_holds_it() {
        let tab = Message::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
            modifiers: iced::keyboard::Modifiers::default(),
        };
        let mut app = app();
        update(&mut app, Message::WorkspaceChosen(Workspace::Paint));
        update(&mut app, tab.clone());
        assert_eq!(app.desk.now, Workspace::Sculpt);
        update(&mut app, tab.clone());
        assert_eq!(app.desk.now, Workspace::Paint);
        update(&mut app, Message::ShortcutsOpened);
        update(&mut app, tab);
        assert_eq!(app.desk.now, Workspace::Paint, "Tab went past a card");
    }

    /// Ctrl with 1 to 4 picks a workspace; the bare digits stay the
    /// library's.
    #[test]
    fn ctrl_and_a_digit_is_a_workspace() {
        for (index, which) in Workspace::ALL.into_iter().enumerate() {
            let digit = (index + 1).to_string();
            assert!(matches!(
                crate::viewport::shortcut(&digit, true, false, false),
                Some(Message::WorkspaceChosen(chosen)) if chosen == which
            ));
            assert!(matches!(
                crate::viewport::shortcut(&digit, false, false, false),
                Some(Message::LibraryKey(at)) if at == index
            ));
        }
    }
}
