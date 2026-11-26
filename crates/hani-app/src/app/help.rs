// SPDX-License-Identifier: AGPL-3.0-only

//! Help: what HaniSculpt is, how to get around it, and how it works, as
//! Markdown files drawn in a card.
//!
//! Asked for by the maintainer on 2026-11-26: *"a high level help document
//! describing what this tool is and does and a broad overview of how it
//! works. Question mark button in the top bar before the other window
//! buttons. Help is in markdown files."*
//!
//! The pages are `docs/help/*.md`, compiled in, so they are there in the
//! browser build too and cannot go missing from an install. A page links to
//! another as `help:<name>`; a link to anything else goes through the one
//! guard every link does (`links.rs`). Parsed when Help opens, not at start.
//!
//! Opened by the `?` in the title bar, before the window buttons, and by the
//! Help menu; closed by its own button or Escape. The keyboard's `?` stays
//! the card of keys.
//!
//! The pages are tabs across the top, each with a line on hover saying what
//! it covers, and a search box finds the sections of every page that say
//! what was typed: the maintainer's second ask of the same day, *"have the
//! help browser use tabs and search to navigate between things. Also
//! tooltips for sections"*. The sections of the panel take a hover line
//! from here too ([`section_tip`]), so that the help and the panel say the
//! same thing of each.

use iced::widget::{button, column, container, markdown, row, scrollable, space, text, text_input};
use iced::{Alignment, Element, Length};

use super::Hani;
use crate::message::{HelpMessage, Message, PanelSection};
use crate::theme;

/// The pages, in the order of their tabs: the name a link uses, the tab's
/// title, the line its tab shows on hover, and the Markdown.
const PAGES: [(&str, &str, &str, &str); 3] = [
    (
        "what-it-is",
        "What it is",
        "What HaniSculpt is for and what it can do",
        include_str!("../../../../docs/help/what-it-is.md"),
    ),
    (
        "getting-around",
        "Getting around",
        "The workspaces, the window, the view, the keys and the bodies",
        include_str!("../../../../docs/help/getting-around.md"),
    ),
    (
        "how-it-works",
        "How it works",
        "The field, the brushes, the mesh, the paint and the files, in broad terms",
        include_str!("../../../../docs/help/how-it-works.md"),
    ),
];

/// The most search results shown.
const MOST_FOUND: usize = 12;

/// What a link to another page starts with.
const PAGE_LINK: &str = "help:";

/// Help, open or not, and the page shown.
#[derive(Debug, Default)]
pub(super) struct HelpState {
    pub open: bool,
    pub page: usize,
    /// What is typed in the search box. Empty shows the page.
    pub query: String,
    /// Each page parsed, once Help has been opened.
    pages: Vec<markdown::Content>,
}

/// One section of one page that says what was searched for: the page, the
/// section's heading, and the line that says it.
#[derive(Debug, Clone, PartialEq)]
struct Found {
    page: usize,
    heading: String,
    line: String,
}

/// The sections of every page that contain every word of `query`, ignoring
/// case: by heading and by line, the heading first.
fn search(query: &str) -> Vec<Found> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (page, (_, title, _, source)) in PAGES.iter().enumerate() {
        let text = body(source);
        let mut heading = title.to_string();
        let mut section = String::new();
        let finish = |heading: &str, section: &str, found: &mut Vec<Found>| {
            let lower = format!("{heading}\n{section}").to_lowercase();
            if words.iter().all(|word| lower.contains(word.as_str())) {
                let line = section
                    .lines()
                    .find(|line| {
                        words.iter().any(|word| line.to_lowercase().contains(word.as_str()))
                    })
                    .unwrap_or_default()
                    .trim()
                    .trim_start_matches("- ")
                    .replace("**", "")
                    .replace('*', "");
                found.push(Found { page, heading: heading.to_string(), line });
            }
        };
        for line in text.lines() {
            if let Some(next) = line.strip_prefix("## ").or_else(|| line.strip_prefix("# ")) {
                finish(&heading, &section, &mut found);
                heading = next.trim().to_string();
                section.clear();
            } else {
                section.push_str(line);
                section.push('\n');
            }
        }
        finish(&heading, &section, &mut found);
    }
    found.retain(|each| !each.line.is_empty() || !each.heading.is_empty());
    found.truncate(MOST_FOUND);
    found
}

/// A page's Markdown without the licence comment each file opens with.
fn body(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("<!--"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The page a `help:` link names.
fn page_named(link: &str) -> Option<usize> {
    let name = link.strip_prefix(PAGE_LINK)?;
    PAGES.iter().position(|(slug, _, _, _)| *slug == name)
}

impl Hani {
    pub(super) fn help_changed(&mut self, message: HelpMessage) {
        match message {
            HelpMessage::Opened => {
                self.top_menu = None;
                if self.help.pages.is_empty() {
                    self.help.pages = PAGES
                        .iter()
                        .map(|(_, _, _, source)| markdown::Content::parse(&body(source)))
                        .collect();
                }
                self.help.open = true;
            }
            HelpMessage::Page(page) => {
                self.help.page = page.min(PAGES.len() - 1);
                // A page chosen, from a tab or a result, is what is shown.
                self.help.query.clear();
            }
            HelpMessage::Searched(query) => self.help.query = query,
            HelpMessage::Link(link) => match page_named(&link) {
                Some(page) => self.help.page = page,
                None => {
                    if let Err(why) = crate::links::open_in_browser(&link) {
                        self.status = why;
                    }
                }
            },
            HelpMessage::Closed => self.help.open = false,
        }
    }

    /// Escape: close Help, and say whether it was open.
    pub(super) fn help_escaped(&mut self) -> bool {
        std::mem::replace(&mut self.help.open, false)
    }

    /// The `?` in the title bar.
    pub(super) fn help_button(&self) -> Element<'_, Message> {
        crate::ui::tip(
            button(text("?").size(theme::TEXT_SIZE).color(theme::TEXT_DIM))
                .padding(iced::Padding { top: theme::S1, bottom: theme::S1, left: 9.0, right: 9.0 })
                .style(theme::window_control)
                .on_press(Message::Help(HelpMessage::Opened)),
            "Help",
        )
    }

    /// The card, over everything below it.
    pub(super) fn help_card(&self) -> Element<'_, Message> {
        let searching = !self.help.query.trim().is_empty();
        let tabs = PAGES.iter().enumerate().fold(
            row![].spacing(theme::S1).width(Length::Fill),
            |tabs, (index, (_, title, tip, _))| {
                tabs.push(crate::ui::tip(
                    crate::ui::toggle(
                        *title,
                        index == self.help.page && !searching,
                        Message::Help(HelpMessage::Page(index)),
                    ),
                    *tip,
                ))
            },
        );
        let page: Element<'_, Message> =
            if searching { self.help_results() } else { self.help_page() };
        let card = container(
            column![
                row![
                    text("Help").size(theme::TITLE_SIZE).color(theme::TEXT),
                    space().width(Length::Fill),
                    text_input("Search help", &self.help.query)
                        .on_input(|query| Message::Help(HelpMessage::Searched(query)))
                        .size(theme::LABEL_SIZE)
                        .padding(theme::S2)
                        .width(Length::Fixed(260.0)),
                    button(text("Close").size(theme::LABEL_SIZE))
                        .padding(theme::PAD_BUTTON)
                        .style(theme::tool_button)
                        .on_press(Message::Help(HelpMessage::Closed)),
                ]
                .spacing(theme::S3)
                .align_y(Alignment::Center),
                tabs,
                scrollable(
                    container(page)
                        .padding(iced::Padding { right: theme::S4, ..iced::Padding::ZERO })
                )
                .height(Length::Fill)
                .width(Length::Fill),
            ]
            .spacing(theme::S4),
        )
        .padding(theme::S5)
        .max_width(980.0)
        .max_height(720.0)
        .style(theme::menu_card);
        super::panel::modal_layer(card)
    }

    /// What the search found: a row a section, which opens its page.
    fn help_results(&self) -> Element<'_, Message> {
        let found = search(&self.help.query);
        if found.is_empty() {
            return text("Nothing found").size(theme::LABEL_SIZE).color(theme::TEXT_MUTE).into();
        }
        found
            .into_iter()
            .fold(column![].spacing(theme::S2), |list, each| {
                let title = PAGES[each.page].1;
                list.push(
                    button(
                        column![
                            text(format!("{title}: {}", each.heading))
                                .size(theme::LABEL_SIZE)
                                .color(theme::TEXT),
                            text(each.line).size(theme::CAPTION_SIZE).color(theme::TEXT_DIM),
                        ]
                        .spacing(2),
                    )
                    .width(Length::Fill)
                    .padding(theme::S2)
                    .style(theme::tool_button)
                    .on_press(Message::Help(HelpMessage::Page(each.page))),
                )
            })
            .into()
    }

    /// The page in hand, drawn.
    fn help_page(&self) -> Element<'_, Message> {
        match self.help.pages.get(self.help.page) {
            Some(content) => markdown::view(
                content.items(),
                markdown::Settings::with_text_size(
                    theme::TEXT_SIZE,
                    markdown::Style {
                        // The application's own faces, which in a browser
                        // are the only ones there are: a span asked for in a
                        // family or a style with no face panics there. Bold
                        // and italic spans keep this family. See
                        // `theme::MONO` and `assets/fonts`.
                        font: theme::FONT,
                        inline_code_font: theme::MONO,
                        code_block_font: theme::MONO,
                        ..markdown::Style::from_palette(theme::theme().palette())
                    },
                ),
            )
            .map(|link| Message::Help(HelpMessage::Link(link))),
            None => space().into(),
        }
    }
}

/// The hover line of the COATS block, which folds as a section does but is
/// not one of [`PanelSection`].
pub(super) const COATS_TIP: &str =
    "Whole coats in one press: base coat, zenithal, sponging, varnish";

/// The hover line of the LAYERS heading.
pub(super) const LAYERS_TIP: &str = "The paint layers: where a stroke goes, and how each blends";

/// What a section of the panel is for, shown when the pointer rests on its
/// heading. A few words each, as the names of the icon buttons are.
pub(super) fn section_tip(which: PanelSection) -> &'static str {
    match which {
        PanelSection::Bodies => "Every body in the document: pick, hide, group and arrange them",
        PanelSection::Brushes => "The brushes, each with a picture of a stroke",
        PanelSection::Workbench => "Try a brush on a ball or a copy, away from the model",
        PanelSection::Add => "Shapes set down by a drag, joined in or a body of their own",
        PanelSection::Brush => "The brush in hand: size, strength, profile and what it does",
        PanelSection::Stroke => "How a stroke is laid down",
        PanelSection::Form => "Thicken, thin or hollow the whole body, or change its detail",
        PanelSection::Extract => "The masked area as a plate of its own",
        PanelSection::Deform => "Twist, bend or taper the active body",
        PanelSection::Revolve => "Draw a line beside an axis and spin it into a body",
        PanelSection::Pattern => "A texture pressed in by whatever brush is in hand",
        PanelSection::History => "The steps that can be undone",
        PanelSection::Detail => "How fine the voxels are, and what that costs",
        PanelSection::Filament => "The filaments loaded in the printer, for a 3MF in colour",
        PanelSection::Export => "Formats, detail and the checks before a print",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> Hani {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        app.welcome = false;
        app
    }

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    #[test]
    fn every_page_has_a_title_and_something_on_it() {
        for (slug, title, _, source) in PAGES {
            let content = markdown::Content::parse(&body(source));
            assert!(content.items().len() > 3, "{slug} is nearly empty");
            assert!(!title.is_empty());
            assert!(!body(source).contains("SPDX"), "{slug} shows its licence comment");
        }
    }

    #[test]
    fn every_page_link_names_a_page() {
        for (slug, _, _, source) in PAGES {
            for (at, _) in source.match_indices("(help:") {
                let name: String = source[at + 1..].chars().take_while(|c| *c != ')').collect();
                assert!(page_named(&name).is_some(), "{slug} links to {name}, which is no page");
            }
            // And a link out of the help is one the application opens.
            for (at, _) in source.match_indices("(https:") {
                let link: String = source[at + 1..].chars().take_while(|c| *c != ')').collect();
                assert!(crate::links::may_be_opened(&link), "{slug} links to {link}, refused");
            }
        }
    }

    #[test]
    fn help_opens_turns_pages_follows_links_and_closes() {
        let mut app = app();
        update(&mut app, Message::Help(HelpMessage::Opened));
        assert!(app.help.open);
        assert_eq!(app.help.pages.len(), PAGES.len());
        drop(app.help_card());
        drop(app.help_button());
        update(&mut app, Message::Help(HelpMessage::Page(2)));
        assert_eq!(app.help.page, 2);
        update(&mut app, Message::Help(HelpMessage::Link("help:getting-around".into())));
        assert_eq!(app.help.page, 1);
        update(&mut app, Message::Help(HelpMessage::Link("https://example.test/".into())));
        assert_eq!(app.help.page, 1, "a link off the help leaves the page as it was");
        assert!(app.help_escaped());
        assert!(!app.help.open);
        assert!(!app.help_escaped(), "a second escape has nothing to close");
    }

    #[test]
    fn search_finds_sections_by_every_word_and_a_result_opens_its_page() {
        let found = search("wash recesses");
        assert!(!found.is_empty(), "nothing found for a wash");
        assert!(found.iter().all(|each| {
            let line = each.line.to_lowercase();
            line.contains("wash") || line.contains("recess")
        }));
        assert!(search("").is_empty());
        assert!(search("zzzz nowhere").is_empty());
        let mut app = app();
        update(&mut app, Message::Help(HelpMessage::Opened));
        update(&mut app, Message::Help(HelpMessage::Searched("voxels".into())));
        drop(app.help_card());
        let page = search("voxels")[0].page;
        update(&mut app, Message::Help(HelpMessage::Page(page)));
        assert_eq!(app.help.page, page);
        assert!(app.help.query.is_empty(), "choosing a result shows its page");
    }

    #[test]
    fn every_section_of_the_panel_has_a_tip() {
        for which in PanelSection::ALL {
            assert!(!section_tip(which).is_empty(), "{which:?}");
        }
    }
}
