use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Line, Modifier, Span, Style},
    widgets::{Paragraph, Wrap},
};

use super::STACKED_WORKSPACE_WIDTH;
use crate::tui::{app::App, ui_widgets::stylized_body_line};

impl App {
    pub(super) fn draw_extract(&self, frame: &mut Frame, area: Rect) {
        let chunks = if area.width < STACKED_WORKSPACE_WIDTH {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
                .split(area)
        };

        self.extract_form.draw(frame, chunks[0], self.theme);

        let preview_lines = vec![
            Line::from(Span::styled(
                "Workflow",
                Style::default()
                    .fg(self.theme.info)
                    .bg(self.theme.panel_bg)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            stylized_body_line("1. `Enter` one or more PDF paths.", self.theme),
            Line::from("2. Choose extractor, page limit, and worker count."),
            stylized_body_line("3. Press `r` to collect an extract preview.", self.theme),
            Line::from(""),
            Line::from(Span::styled(
                "Result Surface",
                Style::default()
                    .fg(self.theme.info)
                    .bg(self.theme.panel_bg)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("The preview opens in Operation."),
            stylized_body_line("Use tab `3` for extracted text and failures.", self.theme),
            stylized_body_line(
                "Use tab `2` for raw extractor logs when verbose/debug is enabled.",
                self.theme,
            ),
        ];
        frame.render_widget(
            Paragraph::new(preview_lines)
                .style(self.theme.panel_style())
                .wrap(Wrap { trim: false })
                .block(self.theme.block("Preview Output")),
            chunks[1],
        );
    }
}
