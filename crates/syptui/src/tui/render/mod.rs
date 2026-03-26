mod extract;
mod header;
mod home;
mod layout;
mod operation;
mod overlay;
mod review;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::Frame,
    widgets::Block,
};

use super::{app::App, model::Screen};

#[cfg(test)]
pub(super) use operation::{StageTimingSnapshot, stage_timing_bars};

const STACKED_SCREEN_WIDTH: u16 = 100;
const STACKED_WORKSPACE_WIDTH: u16 = 140;
const STACKED_REVIEW_WIDTH: u16 = 120;

impl App {
    pub(super) fn draw(&self, frame: &mut Frame) {
        frame.render_widget(Block::default().style(self.theme.app_style()), frame.area());
        let header_height = header::header_height(frame.area().width, self.shortcut_actions());
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(header_height), Constraint::Min(10)])
            .split(frame.area());

        self.draw_header(frame, chunks[0]);
        match self.screen {
            Screen::Home => self.draw_home(frame, chunks[1]),
            Screen::RunForm => self.run_form.draw(frame, chunks[1], self.theme),
            Screen::ExtractForm => self.draw_extract(frame, chunks[1]),
            Screen::Sessions => self.session_view.draw(frame, chunks[1], self.theme),
            Screen::Operation => self.draw_operation(frame, chunks[1]),
            Screen::TaxonomyReview => {
                if let Some((x, y)) = self.draw_taxonomy_review(frame, chunks[1]) {
                    frame.set_cursor_position((x, y));
                }
            }
        }

        if let Some(overlay) = &self.overlay {
            self.draw_overlay(frame, overlay);
        }
    }
}
