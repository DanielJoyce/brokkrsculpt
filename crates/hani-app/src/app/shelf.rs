// SPDX-License-Identifier: AGPL-3.0-only

//! Saving in a browser: the sculpts kept there, and a sculpt to the computer.
//!
//! A browser has no folders the application can write in, so on the web
//! **Save keeps the sculpt in the browser's own storage** (IndexedDB, through
//! `web.rs`) under the sculpt's name, and *Save to computer* hands the same
//! `.hani` bytes over as a download. The card this module draws lists what is
//! kept, with Open, Download and Delete for each, and names a sculpt to save.
//! Opening from the computer is the ordinary Open, through `files.rs`.
//!
//! The storage is asked and answers later, so every change here is two
//! messages: the ask, which returns a task, and the answer. A sculpt counts
//! as saved from the moment its bytes are taken, and as unsaved again if the
//! browser then refuses them, so an edit made while the browser writes is
//! not lost from the count. A save asked for by the unsaved-work prompt goes
//! on to what was asked only when the browser says it has the bytes.
//!
//! On the desktop none of this is offered: the menu has its own Save. The
//! messages still run there, for the tests, and the storage answers that
//! there is none. See `docs/WEB.md`.

use std::sync::Arc;

use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Alignment, Element, Length, Task};

use super::{Hani, PROJECT_EXTENSION, PendingAction};
use crate::message::{AfterFetch, AfterSave, Message, ShelfMessage};
use crate::theme;
use crate::web::Stored;

/// The card's state.
#[derive(Debug, Default)]
pub(crate) struct ShelfState {
    pub open: bool,
    /// The name to save under, as typed.
    pub name: String,
    /// What is kept, newest first, as last listed.
    pub stored: Vec<Stored>,
    /// A delete asked for once, which a second press carries out.
    pub armed: Option<String>,
    /// Whether the browser has been asked to warn before the page is left.
    pub warning: bool,
}

/// A name as typed, made fit to keep: trimmed, without the extension, and
/// not empty.
pub(crate) fn fit_name(typed: &str) -> Option<String> {
    let trimmed = typed.trim();
    let bare = trimmed
        .strip_suffix(&format!(".{PROJECT_EXTENSION}"))
        .unwrap_or(trimmed)
        .trim()
        .chars()
        .take(120)
        .collect::<String>();
    (!bare.is_empty()).then_some(bare)
}

impl Hani {
    /// The sculpt's name, from the file it came from or was kept as.
    fn sculpt_name(&self) -> Option<String> {
        let path = self.project_path.as_ref()?;
        path.file_stem().map(|stem| stem.to_string_lossy().into_owned())
    }

    /// The document as the bytes of a `.hani` file, read back far enough to
    /// know it is this document, as a save on the desktop is.
    pub(super) fn project_bytes(&self) -> Result<Vec<u8>, String> {
        let state = self.project_state();
        let mut bytes = Vec::new();
        hani_core::project::write(&mut bytes, &self.doc, &state)
            .map_err(|error| error.to_string())?;
        self.check_written(&mut bytes.as_slice())?;
        Ok(bytes)
    }

    /// Save in the browser under the sculpt's name. With no name yet, the
    /// card comes up to give it one, unless the unsaved-work prompt asked,
    /// when it is saved under the date and time rather than leave the prompt
    /// waiting on a second card.
    // Called only in a browser, and by the tests on every target.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(super) fn shelf_save(&mut self, then: AfterSave) -> Task<Message> {
        match self.sculpt_name() {
            Some(name) => self.keep(name, then),
            None if then == AfterSave::Continue => self.keep(crate::web::fresh_name(), then),
            None => self.shelf_opened(),
        }
    }

    /// Hand the document's bytes to the browser to keep under `name`.
    fn keep(&mut self, name: String, then: AfterSave) -> Task<Message> {
        let bytes = match self.project_bytes() {
            Ok(bytes) => bytes,
            Err(why) => {
                self.status = format!("could not save {name}: {why}");
                return Task::none();
            }
        };
        self.unsaved = false;
        self.status = format!("saving {name}…");
        let kept = name.clone();
        Task::perform(crate::web::store(name, bytes), move |result| {
            Message::Shelf(ShelfMessage::Saved { name: kept.clone(), result, then })
        })
    }

    /// The card up, and what is kept asked for.
    fn shelf_opened(&mut self) -> Task<Message> {
        self.top_menu = None;
        self.shelf.open = true;
        self.shelf.armed = None;
        self.shelf.name = self.sculpt_name().unwrap_or_else(crate::web::fresh_name);
        self.shelf_listed()
    }

    fn shelf_listed(&self) -> Task<Message> {
        Task::perform(crate::web::stored(), |result| Message::Shelf(ShelfMessage::Listed(result)))
    }

    pub(super) fn shelf_changed(&mut self, message: ShelfMessage) -> Task<Message> {
        match message {
            ShelfMessage::Opened => return self.shelf_opened(),
            ShelfMessage::Closed => self.shelf.open = false,
            ShelfMessage::Listed(Ok(stored)) => self.shelf.stored = stored,
            ShelfMessage::Listed(Err(why)) => {
                self.shelf.stored.clear();
                self.status = format!("could not list what this browser keeps: {why}");
            }
            ShelfMessage::NameTyped(name) => self.shelf.name = name,
            ShelfMessage::SaveHere => match fit_name(&self.shelf.name) {
                Some(name) => return self.keep(name, AfterSave::Stay),
                None => self.status = "could not save: give the sculpt a name".into(),
            },
            ShelfMessage::Saved { name, result, then } => match result {
                Ok(()) => {
                    self.project_path =
                        Some(std::path::PathBuf::from(format!("{name}.{PROJECT_EXTENSION}")));
                    self.status = format!("saved {name} in this browser");
                    self.shelf.open = false;
                    if then == AfterSave::Continue
                        && let Some(action) = self.confirm.take()
                    {
                        return self.run_pending(action);
                    }
                }
                Err(why) => {
                    // The bytes never arrived, so the work is not saved,
                    // and a prompt that asked stays up.
                    self.unsaved = true;
                    self.status = format!("could not save {name} in this browser: {why}");
                }
            },
            ShelfMessage::Open(name) => {
                self.shelf.open = false;
                return self.guard(PendingAction::OpenStored(name));
            }
            ShelfMessage::Download(name) => return self.fetch_kept(name, AfterFetch::Download),
            ShelfMessage::Fetched { name, result, then } => {
                let file = format!("{name}.{PROJECT_EXTENSION}");
                match result {
                    Ok(Some(bytes)) => match then {
                        AfterFetch::Open => {
                            let path = crate::files::hold(&file, bytes.to_vec());
                            self.open_project(&path);
                        }
                        AfterFetch::Download => self.hand_over(&file, &bytes),
                    },
                    Ok(None) => self.status = format!("could not open {name}: it is not kept here"),
                    Err(why) => self.status = format!("could not read {name}: {why}"),
                }
            }
            ShelfMessage::Delete(name) => {
                if self.shelf.armed.as_ref() != Some(&name) {
                    self.shelf.armed = Some(name);
                    return Task::none();
                }
                self.shelf.armed = None;
                let forgotten = name.clone();
                return Task::perform(crate::web::forget(name), move |result| {
                    Message::Shelf(ShelfMessage::Deleted { name: forgotten.clone(), result })
                });
            }
            ShelfMessage::Deleted { name, result } => {
                match result {
                    Ok(()) => self.status = format!("deleted {name} from this browser"),
                    Err(why) => self.status = format!("could not delete {name}: {why}"),
                }
                return self.shelf_listed();
            }
            ShelfMessage::ToComputer => {
                let name = fit_name(&self.shelf.name)
                    .filter(|_| self.shelf.open)
                    .or_else(|| self.sculpt_name())
                    .unwrap_or_else(|| "sculpt".to_string());
                match self.project_bytes() {
                    Ok(bytes) => self.hand_over(&format!("{name}.{PROJECT_EXTENSION}"), &bytes),
                    Err(why) => self.status = format!("could not save {name}: {why}"),
                }
            }
        }
        Task::none()
    }

    /// A kept sculpt asked for, to open or to download.
    pub(super) fn fetch_kept(&mut self, name: String, then: AfterFetch) -> Task<Message> {
        let fetched = name.clone();
        Task::perform(crate::web::fetch(name), move |result| {
            Message::Shelf(ShelfMessage::Fetched {
                name: fetched.clone(),
                result: result.map(|found| found.map(Arc::new)),
                then,
            })
        })
    }

    /// Bytes to the computer as a file of that name: a download.
    fn hand_over(&mut self, file: &str, bytes: &[u8]) {
        let written = crate::files::create(std::path::Path::new(file)).and_then(|mut out| {
            out.write_all(bytes)?;
            out.flush()
        });
        self.status = match written {
            Ok(()) => format!("saved {file} to the computer"),
            Err(error) => format!("could not save {file}: {error}"),
        };
    }

    /// Escape: close the card, and say whether it was open.
    pub(super) fn shelf_escaped(&mut self) -> bool {
        std::mem::replace(&mut self.shelf.open, false)
    }

    /// Ask the browser to warn before the page is left while there is work
    /// not saved, and to stop when there is none.
    pub(super) fn warn_if_unsaved(&mut self) {
        if self.shelf.warning != self.unsaved {
            self.shelf.warning = self.unsaved;
            crate::web::warn_on_leaving(self.unsaved);
        }
    }

    /// The card, over everything below it.
    pub(super) fn shelf_card(&self) -> Element<'_, Message> {
        let send = |message: ShelfMessage| Message::Shelf(message);
        let small = |label: &'static str, message: Option<Message>| {
            button(text(label).size(theme::LABEL_SIZE))
                .padding(theme::PAD_BUTTON)
                .style(theme::tool_button)
                .on_press_maybe(message)
        };
        let saving = row![
            text_input("Name", &self.shelf.name)
                .on_input(move |name| send(ShelfMessage::NameTyped(name)))
                .on_submit(send(ShelfMessage::SaveHere))
                .size(theme::LABEL_SIZE)
                .padding(theme::S2)
                .width(Length::Fill),
            small("Save", fit_name(&self.shelf.name).map(|_| send(ShelfMessage::SaveHere))),
        ]
        .spacing(theme::S2)
        .align_y(Alignment::Center);

        let kept: Element<'_, Message> = if self.shelf.stored.is_empty() {
            text("Nothing kept yet").size(theme::LABEL_SIZE).color(theme::TEXT_MUTE).into()
        } else {
            self.shelf
                .stored
                .iter()
                .fold(column![].spacing(theme::S1), |list, each| {
                    let armed = self.shelf.armed.as_ref() == Some(&each.name);
                    list.push(
                        row![
                            column![
                                text(each.name.as_str()).size(theme::LABEL_SIZE).color(theme::TEXT),
                                text(format!(
                                    "{}  {:.1} MB",
                                    each.when,
                                    each.bytes as f64 / (1024.0 * 1024.0)
                                ))
                                .size(theme::CAPTION_SIZE)
                                .color(theme::TEXT_DIM),
                            ]
                            .spacing(2)
                            .width(Length::Fill),
                            small("Open", Some(send(ShelfMessage::Open(each.name.clone())))),
                            small(
                                "Download",
                                Some(send(ShelfMessage::Download(each.name.clone())))
                            ),
                            small(
                                if armed { "Delete it" } else { "Delete" },
                                Some(send(ShelfMessage::Delete(each.name.clone())))
                            ),
                        ]
                        .spacing(theme::S2)
                        .align_y(Alignment::Center),
                    )
                })
                .into()
        };

        let card = container(
            column![
                row![
                    text("Sculpts in this browser").size(theme::TITLE_SIZE).color(theme::TEXT),
                    space().width(Length::Fill),
                    small("Close", Some(send(ShelfMessage::Closed))),
                ]
                .align_y(Alignment::Center),
                saving,
                scrollable(
                    container(kept)
                        .padding(iced::Padding { right: theme::S4, ..iced::Padding::ZERO })
                )
                .height(Length::Fill)
                .width(Length::Fill),
                row![
                    small("Open from computer…", Some(Message::OpenRequested)),
                    small("Save to computer", Some(send(ShelfMessage::ToComputer))),
                ]
                .spacing(theme::S2),
            ]
            .spacing(theme::S4),
        )
        .padding(theme::S5)
        .max_width(640.0)
        .max_height(560.0)
        .style(theme::menu_card);
        super::panel::modal_layer(card)
    }
}

use std::io::Write;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ConfirmChoice;

    fn app() -> Hani {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        app.welcome = false;
        app
    }

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn shelf(app: &mut Hani, message: ShelfMessage) {
        update(app, Message::Shelf(message));
    }

    #[test]
    fn a_name_is_trimmed_loses_its_extension_and_is_not_empty() {
        assert_eq!(fit_name("  Haniko  ").as_deref(), Some("Haniko"));
        assert_eq!(fit_name("Haniko.hani").as_deref(), Some("Haniko"));
        assert_eq!(fit_name(" .hani "), None);
        assert_eq!(fit_name("   "), None);
        assert_eq!(fit_name(&"x".repeat(300)).map(|name| name.len()), Some(120));
    }

    #[test]
    fn the_bytes_of_a_sculpt_open_as_the_same_sculpt() {
        let mut app = app();
        let bodies = app.doc.body_count();
        let bytes = app.project_bytes().unwrap();
        // Kept, then fetched to open: the bytes come back by name.
        shelf(
            &mut app,
            ShelfMessage::Fetched {
                name: "Kept".into(),
                result: Ok(Some(Arc::new(bytes))),
                then: AfterFetch::Open,
            },
        );
        assert!(app.status.starts_with("opened"), "{}", app.status);
        assert_eq!(app.doc.body_count(), bodies);
        assert_eq!(app.project_path.as_deref(), Some(std::path::Path::new("Kept.hani")));
        assert!(!app.unsaved);
    }

    #[test]
    fn save_with_no_name_brings_the_card_up_and_a_name_saves_under_it() {
        let mut app = app();
        app.unsaved = true;
        assert!(app.project_path.is_none());
        drop(app.shelf_save(AfterSave::Stay));
        assert!(app.shelf.open, "the card did not come up to name it");
        shelf(&mut app, ShelfMessage::NameTyped("  Haniko.hani ".into()));
        shelf(&mut app, ShelfMessage::SaveHere);
        // Counted as saved once the bytes are taken.
        assert!(!app.unsaved);
        shelf(
            &mut app,
            ShelfMessage::Saved { name: "Haniko".into(), result: Ok(()), then: AfterSave::Stay },
        );
        assert_eq!(app.project_path.as_deref(), Some(std::path::Path::new("Haniko.hani")));
        assert!(!app.shelf.open);
        assert!(app.status.contains("saved Haniko"), "{}", app.status);
    }

    #[test]
    fn a_refused_save_leaves_the_work_unsaved_and_the_prompt_up() {
        let mut app = app();
        app.unsaved = true;
        update(&mut app, Message::NewSculpt);
        assert_eq!(app.confirm, Some(PendingAction::NewSculpt));
        drop(app.shelf_save(AfterSave::Continue));
        assert!(!app.shelf.open, "the prompt's save asked for a name");
        shelf(
            &mut app,
            ShelfMessage::Saved {
                name: "Sculpt".into(),
                result: Err("over quota".into()),
                then: AfterSave::Continue,
            },
        );
        assert!(app.unsaved);
        assert_eq!(app.confirm, Some(PendingAction::NewSculpt), "the prompt went away");
        assert!(app.status.contains("over quota"), "{}", app.status);
        // And once it is kept, the prompt's action goes ahead.
        shelf(
            &mut app,
            ShelfMessage::Saved {
                name: "Sculpt".into(),
                result: Ok(()),
                then: AfterSave::Continue,
            },
        );
        assert_eq!(app.confirm, None);
        update(&mut app, Message::ConfirmAnswered(ConfirmChoice::Cancel));
    }

    #[test]
    fn opening_a_kept_sculpt_asks_first_when_there_is_unsaved_work() {
        let mut app = app();
        app.shelf.open = true;
        app.unsaved = true;
        shelf(&mut app, ShelfMessage::Open("Kept".into()));
        assert!(!app.shelf.open);
        assert_eq!(app.confirm, Some(PendingAction::OpenStored("Kept".into())));
    }

    #[test]
    fn delete_wants_a_second_press_and_escape_closes_the_card() {
        let mut app = app();
        drop(app.shelf_opened());
        shelf(&mut app, ShelfMessage::Delete("Kept".into()));
        assert_eq!(app.shelf.armed.as_deref(), Some("Kept"));
        shelf(&mut app, ShelfMessage::Delete("Kept".into()));
        assert_eq!(app.shelf.armed, None);
        assert!(app.shelf_escaped());
        assert!(!app.shelf.open);
    }

    #[test]
    fn the_card_draws_with_and_without_sculpts_kept() {
        let mut app = app();
        drop(app.shelf_opened());
        drop(app.shelf_card());
        app.shelf.stored = vec![Stored { name: "Kept".into(), when: "today".into(), bytes: 5 }];
        app.shelf.armed = Some("Kept".into());
        drop(app.shelf_card());
    }
}
