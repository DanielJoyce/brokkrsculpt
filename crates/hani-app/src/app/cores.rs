// SPDX-License-Identifier: AGPL-3.0-only

//! How many worker threads the application's work is spread over, and a
//! way to measure how many this machine wants.
//!
//! Asked by the maintainer on 2026-11-26: "Are we using all our cores
//! effectively, is there a way to test how many cores we should use on a
//! given PC". Unless told otherwise the pool has one worker for every
//! thread the machine has, which is what the measurements on the build
//! machine favour (`docs/PERFORMANCE.md`): the window's own thread waits
//! while the pool works, so no core needs keeping for it. Where that is
//! not best (hyper-threads, small efficiency cores) *Measure*, in
//! Preferences, times a stroke's kind of work with each count
//! ([`hani_core::calibrate`]) and chooses the fewest threads within a
//! few parts in a hundred of the fastest. The choice is a setting,
//! `cores.conf`, and the pool is sized once, as the application starts.

use hani_core::calibrate::{self, Trial};
use iced::Task;
use iced::widget::{button, pick_list, row, text};
use iced::{Alignment, Element, Length};

use super::Hani;
use crate::message::{CoresMessage, Message};
use crate::theme;

/// The setting's file.
const FILE: &str = "cores.conf";

/// How many threads the machine has.
pub(crate) fn available() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// The count the settings hold, or `None` for one a thread.
pub(crate) fn kept() -> Option<usize> {
    if cfg!(test) {
        return None;
    }
    crate::paths::config_file(FILE)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| {
            crate::paths::entries(&text)
                .find(|(key, _)| *key == "threads")
                .and_then(|(_, value)| value.parse::<usize>().ok())
        })
        .filter(|threads| *threads >= 1)
        .map(|threads| threads.min(available()))
}

fn keep(threads: Option<usize>) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    let path = crate::paths::config_file(FILE).ok_or("no settings directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
    }
    let value = threads.map_or_else(|| "auto".to_string(), |threads| threads.to_string());
    std::fs::write(path, format!("threads = {value}\n")).map_err(|why| why.to_string())
}

/// Size the pool every core's work runs on, from the setting: before the
/// window opens and before anything asks the pool for work. With no
/// setting it is left as it comes, a worker a thread.
pub(crate) fn start_pool() {
    let Some(threads) = kept() else {
        return;
    };
    if let Err(why) = rayon::ThreadPoolBuilder::new().num_threads(threads).build_global() {
        log::warn!("the worker threads were not set to {threads}: {why}");
    }
}

/// One choice in the list: a count, or `None` for one a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadsChoice(pub Option<usize>);

impl std::fmt::Display for ThreadsChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            None => write!(f, "All ({})", available()),
            Some(threads) => write!(f, "{threads}"),
        }
    }
}

/// The choice and a measurement under way.
#[derive(Debug)]
pub(super) struct CoresState {
    pub chosen: Option<usize>,
    pub measuring: bool,
}

impl Default for CoresState {
    fn default() -> Self {
        Self { chosen: kept(), measuring: false }
    }
}

/// Time the workload with each count, off the window's thread.
async fn measured() -> Vec<Trial> {
    let (send, receive) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = send.send(calibrate::measure(&calibrate::counts(available())));
    });
    receive.await.unwrap_or_default()
}

impl Hani {
    pub(super) fn cores_changed(&mut self, message: CoresMessage) -> Task<Message> {
        match message {
            CoresMessage::Chosen(choice) => {
                self.cores.chosen = choice.0;
                self.say_threads_kept(None);
            }
            CoresMessage::Measure => {
                if self.cores.measuring {
                    return Task::none();
                }
                self.cores.measuring = true;
                self.status = "measuring the worker threads".to_string();
                return Task::perform(measured(), |trials| {
                    Message::Cores(CoresMessage::Measured(trials))
                });
            }
            CoresMessage::Measured(trials) => {
                self.cores.measuring = false;
                let Some(best) = calibrate::recommend(&trials) else {
                    self.status = "the worker threads could not be measured".to_string();
                    return Task::none();
                };
                // Every thread is "All", so the setting follows the machine.
                self.cores.chosen = (best < available()).then_some(best);
                let times: Vec<String> = trials
                    .iter()
                    .map(|trial| format!("{}: {:.0} ms", trial.threads, trial.millis))
                    .collect();
                self.say_threads_kept(Some(times.join(", ")));
            }
        }
        Task::none()
    }

    fn say_threads_kept(&mut self, measured: Option<String>) {
        let threads = self.cores.chosen.unwrap_or_else(available);
        let kept = keep(self.cores.chosen);
        self.status = match (kept, measured) {
            (Err(why), _) => format!("the worker threads were not kept: {why}"),
            (Ok(()), Some(times)) => {
                format!("measured {times}; {threads} worker threads from the next start")
            }
            (Ok(()), None) => format!("{threads} worker threads from the next start"),
        };
    }

    /// The row in Preferences.
    pub(super) fn cores_row(&self) -> Element<'_, Message> {
        let choices: Vec<ThreadsChoice> = std::iter::once(ThreadsChoice(None))
            .chain((1..=available()).map(|threads| ThreadsChoice(Some(threads))))
            .collect();
        row![
            text("Worker threads").size(theme::LABEL_SIZE).color(theme::TEXT_DIM),
            pick_list(choices, Some(ThreadsChoice(self.cores.chosen)), |choice| {
                Message::Cores(CoresMessage::Chosen(choice))
            })
            .text_size(theme::LABEL_SIZE)
            .width(Length::Fill),
            crate::ui::tip(
                button(
                    text(if self.cores.measuring { "Measuring" } else { "Measure" })
                        .size(theme::LABEL_SIZE)
                )
                .padding(theme::PAD_CHOICE)
                .style(theme::tool_button)
                .on_press_maybe(
                    (!self.cores.measuring).then_some(Message::Cores(CoresMessage::Measure))
                ),
                "Time a stroke on each number of threads and choose the fastest",
            ),
        ]
        .spacing(theme::S2)
        .align_y(Alignment::Center)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(app: &mut Hani, message: Message) -> Task<Message> {
        app.update(message)
    }

    /// A count chosen is the setting; a measurement chooses the fewest
    /// threads near the fastest, or all of them when that is all; the
    /// row draws.
    #[test]
    fn the_worker_threads_are_chosen_and_measured() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        assert_eq!(app.cores.chosen, None);
        drop(update(&mut app, Message::Cores(CoresMessage::Chosen(ThreadsChoice(Some(1))))));
        assert_eq!(app.cores.chosen, Some(1));
        drop(update(&mut app, Message::Cores(CoresMessage::Measure)));
        assert!(app.cores.measuring);
        let trial = |threads, millis| Trial { threads, millis };
        let all = available();
        drop(update(
            &mut app,
            Message::Cores(CoresMessage::Measured(vec![
                trial(1, 100.0),
                trial(all, 100.0 / all as f32),
            ])),
        ));
        assert!(!app.cores.measuring);
        // Every thread was fastest: "All", which follows the machine.
        assert_eq!(app.cores.chosen, None);
        drop(update(
            &mut app,
            Message::Cores(CoresMessage::Measured(vec![trial(1, 50.0), trial(all.max(2), 60.0)])),
        ));
        assert_eq!(app.cores.chosen, if all > 1 { Some(1) } else { None });
        drop(app.cores_row());
        assert_eq!(ThreadsChoice(Some(3)).to_string(), "3");
    }
}
