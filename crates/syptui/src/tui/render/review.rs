use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::Frame,
    widgets::{Paragraph, Wrap},
};

use super::{
    STACKED_REVIEW_WIDTH,
    layout::{draw_scrolled_panel_with_block, focused_panel_block},
    overlay::draw_text_field,
};
use crate::tui::{
    app::App,
    taxonomy_review::{ReviewPane, TaxonomyReviewView},
    taxonomy_tree::render_section_tree,
    theme::ThemePalette,
    ui_widgets::stylized_body_lines,
};

impl App {
    pub(super) fn draw_taxonomy_review(&self, frame: &mut Frame, area: Rect) -> Option<(u16, u16)> {
        let Some(review) = self.taxonomy_review.as_ref() else {
            return None;
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(5), Constraint::Min(0)])
            .split(area);

        frame.render_widget(
            Paragraph::new(stylized_body_lines(review.status_lines(), self.theme))
                .style(self.theme.panel_style())
                .wrap(Wrap { trim: false })
                .block(self.theme.block("Review Status")),
            chunks[0],
        );

        let content = if chunks[1].width < STACKED_REVIEW_WIDTH {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
                .split(chunks[1])
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1])
        };
        let left = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Min(10)])
            .split(content[0]);

        let cursor = draw_taxonomy_review_suggestion_panel(frame, left[0], review, self.theme);
        draw_scrolled_panel_with_block(
            frame,
            left[1],
            focused_panel_block(
                "History",
                review.focused_pane == ReviewPane::History,
                self.theme,
            ),
            review.history_lines(),
            review.history_scroll,
            "No iteration history yet.",
            self.theme,
        );
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(4), Constraint::Min(0)])
            .split(content[1]);
        frame.render_widget(
            Paragraph::new(stylized_body_lines(
                review.iteration_summary_lines(),
                self.theme,
            ))
            .style(self.theme.panel_style())
            .wrap(Wrap { trim: false })
            .block(self.theme.block("Iteration")),
            right[0],
        );
        let mut tree_state = review.iteration_tree_state.borrow_mut();
        render_section_tree(
            frame,
            right[1],
            focused_panel_block(
                "Iteration Taxonomy",
                review.focused_pane == ReviewPane::IterationTaxonomy,
                self.theme,
            ),
            &review.iteration_taxonomy_sections(),
            &mut tree_state,
            self.theme,
        );

        cursor
    }
}

fn draw_taxonomy_review_suggestion_panel(
    frame: &mut Frame,
    area: Rect,
    review: &TaxonomyReviewView,
    theme: ThemePalette,
) -> Option<(u16, u16)> {
    let block = focused_panel_block(
        "Suggestion",
        review.focused_pane == ReviewPane::Suggestion,
        theme,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return None;
    }

    let mut constraints = vec![Constraint::Min(3)];
    if review.editing {
        constraints.push(Constraint::Length(3));
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    frame.render_widget(
        Paragraph::new(stylized_body_lines(review.suggestion_lines(), theme))
            .style(theme.panel_style())
            .wrap(Wrap { trim: false })
            .scroll((review.focused_scroll().unwrap_or(0), 0)),
        chunks[0],
    );

    if review.editing && chunks.len() > 1 {
        return draw_text_field(frame, chunks[1], "Draft", &review.suggestion_buffer, theme);
    }

    None
}
