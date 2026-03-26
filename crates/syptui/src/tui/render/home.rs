use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Text},
    widgets::{ListItem, Paragraph, Wrap},
};

use super::STACKED_SCREEN_WIDTH;
use crate::tui::{
    app::App,
    ui_widgets::{render_selectable_list, stylized_body_lines},
};

impl App {
    pub(super) fn draw_home(&self, frame: &mut Frame, area: Rect) {
        let chunks = if area.width < STACKED_SCREEN_WIDTH {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(9), Constraint::Min(0)])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
                .split(area)
        };

        let actions = self.home_actions();
        let menu_items = actions
            .iter()
            .map(|item| ListItem::new(item.label()))
            .collect::<Vec<_>>();
        render_selectable_list(
            frame,
            chunks[0],
            self.theme.block("Actions"),
            menu_items,
            Some(self.home_index),
            self.theme,
        );

        let help = Paragraph::new(Text::from(stylized_body_lines(
            [
                "`syp` is the interactive terminal frontend.",
                "",
                "Run Papers: configure and launch the full sorting workflow.",
                "Extract Text: preview raw and LLM-ready text without running the full pipeline.",
                "Sessions: resume, rerun, review, remove, or clear saved runs.",
                "Quit: exit after confirmation.",
            ],
            self.theme,
        )))
        .style(self.theme.panel_style())
        .wrap(Wrap { trim: false })
        .block(self.theme.block("Overview"));
        frame.render_widget(help, chunks[1]);
    }
}
