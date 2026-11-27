// SPDX-License-Identifier: AGPL-3.0-only

//! Save, Save as new and Revert for the brush in hand.
//!
//! Asked for by the maintainer on 2026-11-27: "some kind of per items
//! settings system (brush / tool dirty, save or save as new button)".
//!
//! A brush is taken up from the palette, by its button or its key, and
//! then changed: its size, its profile, what it does, its picture. Before
//! this the panel's heading said `DRAW, CHANGED` and the only way to keep
//! the change was to open *+ Add a brush* on the palette and keep it under
//! a new name. Now the brush remembers which brush it was taken up as, and
//! while it differs from that one, the top of its section offers:
//!
//! * **Save**, over the brush it came from. One of the user's own is
//!   rewritten; one that comes with the application is kept as the user's
//!   own version of it, marked on the palette, and *Restore the one that
//!   came with it* puts the original back, as it already did.
//! * **Save as new…**, under a name typed there.
//! * **Revert**, the brush as it was kept.
//!
//! "Changed" is what the palette already meant by it: the brush in hand is
//! none of the palette's (`Hani::lit`). A brush that says nothing of its
//! size, as most that come with the application do, is not changed by a
//! change of size, and saving it over itself does not start saying one.
//!
//! The brushes that add are brushes on the palette like the rest, so the
//! same three buttons stand at the top of ADD. The tools that are not
//! brushes on the palette (the mask, the cut, the deformers) have no kept
//! versions to differ from, and are not covered.

use iced::widget::{row, text_input};
use iced::{Element, Length};

use super::Hani;
use super::presets::{Place, Preset};
use crate::message::{KeepMessage, Message, StudioMessage};
use crate::theme;

/// Which brush is in hand as, and the name being typed for a new one.
#[derive(Debug, Default)]
pub(super) struct KeepingState {
    /// The palette brush last taken up or kept, by name.
    pub from: Option<String>,
    /// The name typed for Save as new, while it is open.
    pub naming: Option<String>,
}

impl Hani {
    /// The palette brush the brush in hand was taken up as, if it has
    /// changed since: its place on the palette, and it.
    ///
    /// The one last taken up, if it is still there, and otherwise the
    /// classic brush of the family in hand, which is what a brush chosen by
    /// its family's own button is.
    pub(super) fn changed_from(&self) -> Option<(usize, Preset, Place)> {
        if self.tool == super::Tool::Mask {
            return None;
        }
        let now = self.preset_now(String::new());
        let palette = self.studio.presets.palette();
        if Self::lit(&palette, &now).is_some() {
            return None;
        }
        let named = |name: &str| palette.iter().position(|(preset, _)| preset.name == name);
        let at = self
            .kept_as
            .from
            .as_deref()
            .and_then(named)
            .or_else(|| named(self.brush.kind.label()))?;
        let (preset, place) = palette[at];
        Some((at, preset.clone(), place))
    }

    pub(super) fn keep_changed(&mut self, message: KeepMessage) {
        match message {
            KeepMessage::Save => {
                let Some((_, from, _)) = self.changed_from() else {
                    return;
                };
                let mut preset = self.preset_now(from.name.clone());
                // What the kept brush left to the hand, it leaves still.
                if from.size_mm.is_none() {
                    preset.size_mm = None;
                }
                if from.strength.is_none() {
                    preset.strength = None;
                }
                if preset.add.is_some_and(|setup| setup.what == super::adding::Kind::Body) {
                    self.status = "a copy of a body cannot be kept as a brush: the body is this \
                                   document's"
                        .to_string();
                    return;
                }
                if !self.studio.presets.put(preset.clone()) {
                    self.status = "no room for another brush: forget one first".to_string();
                    return;
                }
                self.kept_as.from = Some(from.name.clone());
                self.status = match super::presets::Presets::write(&preset) {
                    Ok(()) => format!("brush saved: {}", from.name),
                    Err(why) => format!("brush saved until the application closes: {why}"),
                };
            }
            KeepMessage::SaveAsOpened => {
                let suggested = self
                    .changed_from()
                    .map(|(_, from, _)| from.name)
                    .or_else(|| self.lit_name())
                    .map(|name| format!("{name} 2"))
                    .unwrap_or_default();
                self.kept_as.naming = Some(suggested);
            }
            KeepMessage::SaveAsNamed(name) => {
                if self.kept_as.naming.is_some() {
                    self.kept_as.naming = Some(name.chars().take(96).collect());
                }
            }
            KeepMessage::SaveAsKept => {
                let Some(name) = self.kept_as.naming.clone() else {
                    return;
                };
                // Through the palette's own Keep, which names, refuses and
                // writes as it always has.
                self.studio.preset_name = name;
                drop(self.studio_changed(StudioMessage::PresetSaved));
                // Kept when the name was taken: the field went empty.
                if self.studio.preset_name.is_empty() {
                    self.kept_as.naming = None;
                }
            }
            KeepMessage::SaveAsClosed => self.kept_as.naming = None,
            KeepMessage::Revert => {
                if let Some((at, from, _)) = self.changed_from() {
                    drop(self.studio_changed(StudioMessage::PresetChosen(at)));
                    self.status = format!("brush back as it was kept: {}", from.name);
                }
            }
        }
    }

    /// The name of the palette brush in hand, unchanged.
    fn lit_name(&self) -> Option<String> {
        let now = self.preset_now(String::new());
        let palette = self.studio.presets.palette();
        Self::lit(&palette, &now).map(|at| palette[at].0.name.clone())
    }

    /// Save, Save as new and Revert, the first and last pressable while the
    /// brush in hand has changed; the name field while Save as new is open.
    ///
    /// **Always there for a brush, and not only once it has changed**: seen
    /// on a screen, a row that came and went moved every control under it
    /// by a row's height, under the pointer that was about to press one.
    pub(super) fn keeping_bar(&self) -> Option<Element<'_, Message>> {
        if self.tool == super::Tool::Mask {
            return None;
        }
        let keep = |message: KeepMessage| Message::Keep(message);
        if let Some(name) = &self.kept_as.naming {
            return Some(
                row![
                    text_input("name the new brush", name)
                        .on_input(move |name| keep(KeepMessage::SaveAsNamed(name)))
                        .on_submit(keep(KeepMessage::SaveAsKept))
                        .padding(theme::PAD_FIELD)
                        .size(theme::LABEL_SIZE)
                        .width(Length::Fill),
                    crate::ui::button_secondary("Keep", Some(keep(KeepMessage::SaveAsKept))),
                    crate::ui::button_secondary("Cancel", Some(keep(KeepMessage::SaveAsClosed))),
                ]
                .spacing(theme::S1)
                .into(),
            );
        }
        let changed = self.changed_from();
        let place = changed.as_ref().map(|(_, _, place)| *place);
        let save = crate::ui::tip(
            crate::ui::button_secondary("Save", place.map(|_| keep(KeepMessage::Save))),
            match place {
                Some(Place::Core) => "Keep these settings as your version of this brush",
                _ => "Keep these settings in this brush",
            },
        );
        Some(
            row![
                save,
                crate::ui::button_secondary("Save as new…", Some(keep(KeepMessage::SaveAsOpened))),
                crate::ui::button_secondary("Revert", place.map(|_| keep(KeepMessage::Revert))),
            ]
            .spacing(theme::S1)
            .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::StudioMessage;
    use hani_core::brush::compound::Actions;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    fn choose(app: &mut Hani, name: &str) {
        let at = app
            .studio
            .presets
            .palette()
            .iter()
            .position(|(preset, _)| preset.name == name)
            .expect("the brush is on the palette");
        update(app, Message::Studio(StudioMessage::PresetChosen(at)));
    }

    fn place_of(app: &Hani, name: &str) -> Option<Place> {
        app.studio
            .presets
            .palette()
            .iter()
            .find(|(preset, _)| preset.name == name)
            .map(|(_, place)| *place)
    }

    /// Something only a changed brush does: what it is made of.
    fn change(app: &mut Hani) {
        let actions = Actions { push: 0.4, inflate: 0.3, ..Actions::default() };
        update(app, Message::Studio(StudioMessage::ActionsChosen(actions)));
    }

    #[test]
    fn a_brush_taken_up_and_changed_says_so_and_offers_to_keep_it() {
        let mut app = app();
        choose(&mut app, "Clay");
        assert!(app.changed_from().is_none(), "a brush just taken up is not changed");
        // The row stands, so that nothing under it moves when it changes.
        assert!(app.keeping_bar().is_some());
        change(&mut app);
        let (_, from, place) = app.changed_from().expect("the change was not seen");
        assert_eq!(from.name, "Clay");
        assert_eq!(place, Place::Core);
        assert!(app.keeping_bar().is_some());
        assert!(app.live_brush_name().ends_with("CHANGED"), "{}", app.live_brush_name());

        // Revert puts it back as it was kept.
        update(&mut app, Message::Keep(KeepMessage::Revert));
        assert!(app.changed_from().is_none(), "Revert left it changed");
        assert_eq!(app.live_brush_name(), "CLAY");
    }

    #[test]
    fn save_over_a_brush_that_came_keeps_the_users_version_of_it() {
        let mut app = app();
        choose(&mut app, "Clay");
        change(&mut app);
        update(&mut app, Message::Keep(KeepMessage::Save));
        assert!(app.status.starts_with("brush saved: Clay"), "{}", app.status);
        assert_eq!(place_of(&app, "Clay"), Some(Place::Changed));
        assert!(app.changed_from().is_none(), "saved, it is still changed");
        assert_eq!(app.live_brush_name(), "CLAY");
        // The size was the hand's before, and is still.
        let kept = app.studio.presets.list.iter().find(|preset| preset.name == "Clay").unwrap();
        assert_eq!(kept.size_mm, None);
    }

    #[test]
    fn save_as_new_keeps_a_brush_under_the_name_typed_and_takes_it_up() {
        let mut app = app();
        choose(&mut app, "Draw");
        change(&mut app);
        update(&mut app, Message::Keep(KeepMessage::SaveAsOpened));
        assert_eq!(app.kept_as.naming.as_deref(), Some("Draw 2"));
        update(&mut app, Message::Keep(KeepMessage::SaveAsNamed("Puffy draw".to_string())));
        update(&mut app, Message::Keep(KeepMessage::SaveAsKept));
        assert_eq!(app.kept_as.naming, None, "the name field is still open");
        assert_eq!(place_of(&app, "Puffy draw"), Some(Place::Own));
        assert_eq!(place_of(&app, "Draw"), Some(Place::Core), "the brush it came from changed");
        assert!(app.changed_from().is_none());
        assert_eq!(app.live_brush_name(), "PUFFY DRAW");

        // Changed again, Save is over the new one.
        change(&mut app);
        let actions = Actions { push: 0.9, ..Actions::default() };
        update(&mut app, Message::Studio(StudioMessage::ActionsChosen(actions)));
        assert_eq!(app.changed_from().map(|(_, from, _)| from.name), Some("Puffy draw".into()));
    }

    #[test]
    fn save_as_new_without_a_name_stays_open_and_cancel_closes_it() {
        let mut app = app();
        choose(&mut app, "Draw");
        change(&mut app);
        update(&mut app, Message::Keep(KeepMessage::SaveAsOpened));
        update(&mut app, Message::Keep(KeepMessage::SaveAsNamed("  ".to_string())));
        update(&mut app, Message::Keep(KeepMessage::SaveAsKept));
        assert!(app.kept_as.naming.is_some(), "a nameless brush closed the field");
        assert!(app.status.contains("name"), "{}", app.status);
        update(&mut app, Message::Keep(KeepMessage::SaveAsClosed));
        assert!(app.kept_as.naming.is_none());
        assert!(app.changed_from().is_some(), "cancel lost the change");
    }

    #[test]
    fn the_mask_is_not_a_brush_to_keep() {
        let mut app = app();
        choose(&mut app, "Draw");
        change(&mut app);
        update(&mut app, Message::ToolChanged(super::super::Tool::Mask));
        assert!(app.changed_from().is_none());
        assert!(app.keeping_bar().is_none());
    }
}
