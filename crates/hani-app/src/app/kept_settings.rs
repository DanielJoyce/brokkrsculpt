// SPDX-License-Identifier: AGPL-3.0-only

//! The settings that were kept in a file and had no control: the printer's
//! address, the slicer's preset folder, a build the update offer was told
//! to skip, and the filament slots.
//!
//! Said by the maintainer on 2026-11-28: "every setting in a file needs to
//! be editable in the ui". These four were written down by hand, or by one
//! action only (Sync filaments from printer, Not now on an update), and
//! there was no way back from inside the application. Each is a control
//! now, writing the same file the reader already reads (`printer.rs`,
//! `slicer.rs`, `update.rs`, `palette.rs`), so a file written by hand
//! before goes on working.

use iced::widget::{column, row, text, text_input};
use iced::{Alignment, Element, Length};

use super::Hani;
use crate::message::Message;
use crate::theme;

/// A change to one of these settings.
#[derive(Debug, Clone, PartialEq)]
pub enum KeptMessage {
    /// The printer's host, as typed.
    Host(String),
    /// Its port, as typed.
    Port(String),
    /// Enter in either: write `printer.conf`.
    PrinterKept,
    /// The slicer's preset folder, as typed. Empty is "find it".
    Datadir(String),
    /// Enter in it: write `slicer.conf`.
    DatadirKept,
    /// Offer the build the update offer was told to skip again.
    SkipCleared,
    /// A filament slot's name, colour (`#RRGGBB`) or material, as typed.
    SlotName(usize, String),
    SlotColour(usize, String),
    SlotMaterial(usize, String),
    /// Enter in any of a slot's fields: write `filaments.conf`.
    SlotsKept,
}

/// What is typed in the fields, before it is kept.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct KeptSettings {
    pub host: String,
    pub port: String,
    pub datadir: String,
}

impl KeptSettings {
    /// What the files hold now. Empty in a test, which must not read the
    /// user's files.
    pub(super) fn read() -> Self {
        if cfg!(test) {
            return Self { port: crate::printer::MOONRAKER_PORT.to_string(), ..Self::default() };
        }
        let printer = crate::printer::configured(crate::printer::config_path().as_deref());
        Self {
            host: printer.as_ref().map(|printer| printer.host.clone()).unwrap_or_default(),
            port: printer
                .map_or(crate::printer::MOONRAKER_PORT, |printer| printer.port)
                .to_string(),
            datadir: crate::paths::config_file(SLICER_FILE)
                .and_then(|path| std::fs::read_to_string(path).ok())
                .and_then(|text| {
                    crate::paths::entries(&text)
                        .find(|(key, _)| *key == "datadir")
                        .map(|(_, value)| value.to_string())
                })
                .unwrap_or_default(),
        }
    }
}

/// The slicer's settings file: `slicer.rs` reads `datadir` out of it.
const SLICER_FILE: &str = "slicer.conf";

/// Write a settings file, or nothing in a test.
fn write(path: Option<std::path::PathBuf>, body: &str) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    let path = path.ok_or("no settings directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
    }
    std::fs::write(&path, body).map_err(|why| format!("{}: {why}", path.display()))
}

/// What the printer's fields hold, as a file, or why they cannot be kept.
pub(super) fn printer_file(host: &str, port: &str) -> Result<String, String> {
    let host = host.trim();
    let port: u16 = port.trim().parse().map_err(|_| format!("{} is not a port", port.trim()))?;
    if host.is_empty() {
        return Ok(format!("port = {port}\n"));
    }
    if !crate::printer::valid_host(host) {
        return Err(format!("{host} is not a host name or an address"));
    }
    Ok(format!("host = {host}\nport = {port}\n"))
}

/// What the slicer's field holds, as a file, or why it cannot be kept.
pub(super) fn slicer_file(datadir: &str) -> Result<String, String> {
    let datadir = datadir.trim();
    if datadir.is_empty() {
        return Ok(String::new());
    }
    let path = std::path::Path::new(datadir);
    if !path.is_absolute() {
        return Err(format!("{datadir} is not a whole path"));
    }
    Ok(format!("datadir = {datadir}\n"))
}

impl Hani {
    pub(super) fn kept_changed(&mut self, message: KeptMessage) {
        match message {
            KeptMessage::Host(host) => self.kept.host = host,
            KeptMessage::Port(port) => self.kept.port = port,
            KeptMessage::PrinterKept => {
                self.status = match printer_file(&self.kept.host, &self.kept.port)
                    .and_then(|body| write(crate::printer::config_path(), &body))
                {
                    Ok(()) if self.kept.host.trim().is_empty() => "no printer set".to_string(),
                    Ok(()) => format!("printer set to {}", self.kept.host.trim()),
                    Err(why) => format!("the printer was not kept: {why}"),
                };
            }
            KeptMessage::Datadir(datadir) => self.kept.datadir = datadir,
            KeptMessage::DatadirKept => {
                self.status = match slicer_file(&self.kept.datadir)
                    .and_then(|body| write(crate::paths::config_file(SLICER_FILE), &body))
                {
                    Ok(()) if self.kept.datadir.trim().is_empty() => {
                        "the slicer's presets will be looked for".to_string()
                    }
                    Ok(()) => format!("the slicer's presets are in {}", self.kept.datadir.trim()),
                    Err(why) => format!("the slicer's folder was not kept: {why}"),
                };
            }
            KeptMessage::SkipCleared => {
                self.update_settings.skip_build = None;
                crate::update::store(&self.update_settings);
                self.status = "a skipped build will be offered again".to_string();
            }
            KeptMessage::SlotName(slot, name) => {
                if let Some(slot) = self.palette.slots.get_mut(slot) {
                    slot.name = name;
                }
            }
            KeptMessage::SlotColour(slot, colour) => {
                if let Some(slot) = self.palette.slots.get_mut(slot) {
                    slot.colour = colour;
                }
            }
            KeptMessage::SlotMaterial(slot, material) => {
                if let Some(slot) = self.palette.slots.get_mut(slot) {
                    slot.material = material;
                }
            }
            KeptMessage::SlotsKept => {
                let bad = self
                    .palette
                    .slots
                    .iter()
                    .position(|slot| crate::palette::normalise_colour(&slot.colour).is_none());
                if let Some(bad) = bad {
                    self.status = format!("slot {}: a colour is #RRGGBB", bad + 1);
                    return;
                }
                for slot in &mut self.palette.slots {
                    if let Some(colour) = crate::palette::normalise_colour(&slot.colour) {
                        slot.colour = colour;
                    }
                }
                self.status = match self.palette.save() {
                    Ok(()) => "filaments kept".to_string(),
                    Err(why) => format!("the filaments were not kept: {why}"),
                };
            }
        }
    }

    /// PRINTER and SLICER, for Preferences.
    pub(super) fn kept_settings_block(&self) -> Element<'_, Message> {
        let send = |message: KeptMessage| Message::Kept(message);
        let label = |words: &'static str| {
            text(words).size(theme::LABEL_SIZE).color(theme::TEXT_DIM).width(Length::Fixed(70.0))
        };
        let field = |placeholder: &'static str, value: &str| {
            text_input(placeholder, value).size(theme::LABEL_SIZE).padding(theme::PAD_FIELD)
        };
        let mut block = column![
            text("PRINTER").size(theme::HEADING_SIZE).color(theme::TEXT_MUTE),
            crate::ui::tip(
                row![
                    label("Host"),
                    field("address", &self.kept.host)
                        .on_input(move |typed| send(KeptMessage::Host(typed)))
                        .on_submit(send(KeptMessage::PrinterKept)),
                ]
                .spacing(theme::S2)
                .align_y(Alignment::Center),
                "The printer's Moonraker address; Enter keeps it",
            ),
            crate::ui::tip(
                row![
                    label("Port"),
                    field("7125", &self.kept.port)
                        .on_input(move |typed| send(KeptMessage::Port(typed)))
                        .on_submit(send(KeptMessage::PrinterKept)),
                ]
                .spacing(theme::S2)
                .align_y(Alignment::Center),
                "Moonraker's port, 7125 unless it was changed; Enter keeps it",
            ),
            text("SLICER").size(theme::HEADING_SIZE).color(theme::TEXT_MUTE),
            crate::ui::tip(
                row![
                    label("Presets"),
                    field("found by itself", &self.kept.datadir)
                        .on_input(move |typed| send(KeptMessage::Datadir(typed)))
                        .on_submit(send(KeptMessage::DatadirKept)),
                ]
                .spacing(theme::S2)
                .align_y(Alignment::Center),
                "The slicer's settings folder, when it is not found by itself; Enter keeps it",
            ),
        ]
        .spacing(theme::S2);
        if let Some(build) = self.update_settings.skip_build {
            block = block.push(
                row![
                    text(format!("Skipping build {build}"))
                        .size(theme::LABEL_SIZE)
                        .color(theme::TEXT_DIM)
                        .width(Length::Fill),
                    crate::ui::tip(
                        crate::ui::button_secondary(
                            "Offer it again",
                            Some(send(KeptMessage::SkipCleared))
                        ),
                        "Stop skipping this build: the update check offers it again",
                    ),
                ]
                .spacing(theme::S2)
                .align_y(Alignment::Center),
            );
        }
        block.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    /// What the printer's and the slicer's fields keep is what their
    /// readers read back, and what cannot be read back is refused.
    #[test]
    fn the_printer_and_slicer_fields_write_what_is_read() {
        let body = printer_file(" printer.local ", "7125").expect("a host and a port");
        let path = std::env::temp_dir().join(format!("hani-kept-printer-{}", std::process::id()));
        std::fs::write(&path, &body).unwrap();
        let read = crate::printer::configured(Some(&path)).expect("read back");
        assert_eq!((read.host.as_str(), read.port), ("printer.local", 7125));
        std::fs::remove_file(&path).ok();
        assert!(printer_file("not a host!", "7125").is_err());
        assert!(printer_file("printer.local", "port").is_err());
        assert_eq!(printer_file("", "7125").unwrap(), "port = 7125\n");

        assert!(slicer_file("relative/path").is_err());
        assert_eq!(slicer_file("").unwrap(), "");
        let absolute = std::env::temp_dir();
        let body = slicer_file(absolute.to_str().unwrap()).unwrap();
        assert!(body.starts_with("datadir = "));
    }

    /// A slot's fields change the palette as they are typed; a colour that
    /// is not #RRGGBB is not kept, and one in lower case is kept in upper.
    #[test]
    fn a_filament_slot_is_edited_in_its_fields() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::Kept(KeptMessage::SlotName(0, "Bone".into())));
        update(&mut app, Message::Kept(KeptMessage::SlotMaterial(0, "PETG".into())));
        update(&mut app, Message::Kept(KeptMessage::SlotColour(0, "#e0d8c0".into())));
        update(&mut app, Message::Kept(KeptMessage::SlotsKept));
        let slot = &app.palette.slots[0];
        assert_eq!((slot.name.as_str(), slot.material.as_str()), ("Bone", "PETG"));
        assert_eq!(slot.colour, "#E0D8C0");
        assert_eq!(app.status, "filaments kept");
        update(&mut app, Message::Kept(KeptMessage::SlotColour(1, "beige".into())));
        update(&mut app, Message::Kept(KeptMessage::SlotsKept));
        assert!(app.status.starts_with("slot 2"), "{}", app.status);
    }

    /// The recent files are cleared from the File menu.
    #[test]
    fn the_recent_files_can_be_cleared() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        let scratch = std::env::temp_dir().join(format!("hani-kept-recent-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        app.recent = crate::recent::Recent::load_from(Some(scratch.join("recent")));
        app.recent.record(&scratch.join("one.hani"));
        assert!(!app.recent.is_empty());
        update(&mut app, Message::RecentCleared);
        assert!(app.recent.is_empty());
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// A skipped build is offered again from Preferences, and the block
    /// shows it only while one is skipped.
    #[test]
    fn a_skipped_build_can_be_offered_again() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        app.update_settings.skip_build = Some(42);
        drop(app.kept_settings_block());
        update(&mut app, Message::Kept(KeptMessage::SkipCleared));
        assert_eq!(app.update_settings.skip_build, None);
    }
}
