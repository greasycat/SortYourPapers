use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{Frame, Line, Modifier, Span, Style},
    widgets::{Paragraph, Wrap},
};

use super::super::{app::App, model::Screen};
use crate::tui::theme::ThemePalette;

pub(super) fn header_height(width: u16, actions: &[(&str, &str)]) -> u16 {
    let inner_width = usize::from(width.saturating_sub(2)).max(1);
    let hint_lines = wrapped_line_count_from_len(shortcut_hint_len(actions), inner_width);
    if hint_lines <= 1 {
        3
    } else {
        (hint_lines as u16 + 3).min(5)
    }
}

impl App {
    pub(super) fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let actions = self.shortcut_actions();
        let title = match self.screen {
            Screen::Home => "Home",
            Screen::RunForm => "Run Configuration",
            Screen::ExtractForm => "Extract Text",
            Screen::Sessions => "Sessions",
            Screen::Operation => &self.operation.title,
            Screen::TaxonomyReview => "Taxonomy Review",
        };
        let block = self.theme.block("");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let title_line = Line::from(vec![
            Span::raw(format!(" {title}")),
            Span::raw(" "),
            Span::styled(
                format!("[{}]", self.operation.state.label()),
                Style::default()
                    .fg(self.theme.status_color(self.operation.state))
                    .bg(self.theme.panel_bg)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);

        if inner.height > 1 {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(inner);
            frame.render_widget(Paragraph::new(title_line), rows[0]);
            frame.render_widget(
                Paragraph::new(vec![Line::from(shortcut_chip_spans(actions, self.theme))])
                    .style(self.theme.panel_style())
                    .wrap(Wrap { trim: false }),
                rows[1],
            );
            return;
        }

        let left_width = (title.chars().count() + self.operation.state.label().chars().count() + 8)
            .min(inner.width.saturating_sub(1) as usize) as u16;
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(left_width), Constraint::Min(1)])
            .split(inner);

        frame.render_widget(
            Paragraph::new(title_line).style(self.theme.panel_style()),
            chunks[0],
        );
        frame.render_widget(
            Paragraph::new(Line::from(shortcut_chip_spans(actions, self.theme)))
                .style(self.theme.panel_style())
                .alignment(Alignment::Right),
            chunks[1],
        );
    }

    pub(super) fn shortcut_actions(&self) -> &'static [(&'static str, &'static str)] {
        match self.screen {
            Screen::Home => &[
                ("↑/↓", "move"),
                ("Enter", "open"),
                ("t", "theme"),
                ("Esc", "quit"),
            ],
            Screen::RunForm => &[
                ("↑/↓ or j/k", "move"),
                ("←/→ or h/l", "column"),
                ("Enter", "edit/run"),
                ("Space", "toggle/run"),
                ("r", "run"),
                ("s", "save"),
                ("t", "theme"),
                ("Esc", "back"),
            ],
            Screen::ExtractForm => &[
                ("↑/↓ or j/k", "move"),
                ("←/→ or h/l", "cycle"),
                ("Enter", "edit"),
                ("r", "preview"),
                ("t", "theme"),
                ("Esc", "back"),
            ],
            Screen::Sessions => &[
                ("1-5", "filter"),
                ("Tab/h/l", "preview tab"),
                ("PgUp/PgDn", "preview scroll"),
                ("↑/↓", "select"),
                ("p/a", "resume"),
                ("r/x", "rerun"),
                ("v", "review"),
                ("d", "delete"),
                ("c", "clear"),
                ("g", "refresh"),
                ("t", "theme"),
                ("Esc", "back"),
            ],
            Screen::Operation => &[
                ("Tab/h/l", "switch"),
                ("1-4", "jump tab"),
                ("j/k", "scroll"),
                ("Space", "fold"),
                ("PgUp/PgDn", "page"),
                ("g/G", "start/end"),
                ("s", "sessions"),
                ("t", "theme"),
                ("Esc", "back when idle"),
            ],
            Screen::TaxonomyReview => self
                .taxonomy_review
                .as_ref()
                .map(|review| review.shortcut_actions())
                .unwrap_or(&[("Tab/h/l", "change pane"), ("j/k", "scroll")]),
        }
    }
}

fn shortcut_chip_spans(actions: &[(&str, &str)], theme: ThemePalette) -> Vec<Span<'static>> {
    let mut spans = Vec::with_capacity(actions.len() * 2);
    for (index, (key, action)) in actions.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }

        spans.push(Span::styled(
            format!(" {key}: {action} "),
            theme.chip_style(index),
        ));
    }

    spans
}

fn shortcut_hint_len(actions: &[(&str, &str)]) -> usize {
    actions
        .iter()
        .enumerate()
        .map(|(index, (key, action))| {
            key.chars().count() + action.chars().count() + if index == 0 { 4 } else { 5 }
        })
        .sum()
}

fn wrapped_line_count_from_len(line_len: usize, width: usize) -> usize {
    if width == 0 {
        return 1;
    }

    line_len.max(1).div_ceil(width)
}
