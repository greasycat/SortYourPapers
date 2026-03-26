use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Line, Span, Style, Text},
    widgets::{Clear, ListItem, Paragraph, Wrap},
};

use super::{
    STACKED_REVIEW_WIDTH,
    layout::{centered_rect, compact_overlay_rect, overlay_block},
};
use crate::{
    session::commands::describe_rerun_impact,
    tui::{
        app::App,
        model::Overlay,
        session_view::rerun_stage_name,
        theme::ThemePalette,
        ui_widgets::{muted_style, render_selectable_list, stylized_body_lines},
    },
};

impl App {
    pub(super) fn draw_overlay(&self, frame: &mut Frame, overlay: &Overlay) {
        let area = match overlay {
            Overlay::EditField { .. }
            | Overlay::SelectPath { .. }
            | Overlay::SelectRerunStage { .. } => centered_rect(70, 60, frame.area()),
            Overlay::Confirm { title, message, .. } => compact_overlay_rect(
                frame.area(),
                title,
                &[
                    message.as_str(),
                    "",
                    "`Enter` or `y` confirm",
                    "`Esc` cancel",
                ],
            ),
            Overlay::Notice { title, message } => compact_overlay_rect(
                frame.area(),
                title,
                &[message.as_str(), "", "`Enter` or `Esc` dismiss"],
            ),
        };
        frame.render_widget(Clear, area);

        match overlay {
            Overlay::EditField { label, buffer } => {
                if let Some((x, y)) = self.draw_edit_field_overlay(frame, area, label, buffer) {
                    frame.set_cursor_position((x, y));
                }
            }
            Overlay::Confirm { title, message, .. } => {
                let widget = Paragraph::new(Text::from(stylized_body_lines(
                    [
                        message.as_str(),
                        "",
                        "`Enter` or `y` confirm",
                        "`Esc` cancel",
                    ],
                    self.theme,
                )))
                .style(self.theme.panel_style())
                .wrap(Wrap { trim: false })
                .block(overlay_block(title, self.theme));
                frame.render_widget(widget, area);
            }
            Overlay::Notice { title, message } => {
                let widget = Paragraph::new(Text::from(stylized_body_lines(
                    [message.as_str(), "", "`Enter` or `Esc` dismiss"],
                    self.theme,
                )))
                .style(self.theme.panel_style())
                .wrap(Wrap { trim: false })
                .block(overlay_block(title, self.theme));
                frame.render_widget(widget, area);
            }
            Overlay::SelectPath {
                label,
                buffer,
                directories,
                selected,
            } => {
                if let Some((x, y)) = self.draw_select_path_overlay(
                    frame,
                    area,
                    label,
                    buffer,
                    directories,
                    *selected,
                ) {
                    frame.set_cursor_position((x, y));
                }
            }
            Overlay::SelectRerunStage {
                stages,
                selected,
                config,
                ..
            } => {
                let chunks = if area.width < STACKED_REVIEW_WIDTH {
                    Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
                        .split(area)
                } else {
                    Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
                        .split(area)
                };

                let lines = stages
                    .iter()
                    .map(|stage| {
                        ListItem::new(format!(
                            "{} {}",
                            rerun_stage_name(*stage),
                            stage.description()
                        ))
                    })
                    .collect::<Vec<_>>();
                render_selectable_list(
                    frame,
                    chunks[0],
                    self.theme.block("Select Rerun Stage"),
                    lines,
                    Some(*selected),
                    self.theme,
                );

                let impact_lines = stages
                    .get(*selected)
                    .and_then(|stage| describe_rerun_impact(config, *stage).ok())
                    .map(|impact| impact.lines())
                    .unwrap_or_else(|| vec!["Could not describe rerun impact.".to_string()]);
                frame.render_widget(
                    Paragraph::new(stylized_body_lines(impact_lines, self.theme))
                        .style(self.theme.panel_style())
                        .wrap(Wrap { trim: false })
                        .block(self.theme.block("Rerun Consequences")),
                    chunks[1],
                );
            }
        }
    }

    fn draw_edit_field_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        label: &str,
        buffer: &str,
    ) -> Option<(u16, u16)> {
        let block = self.theme.block("Edit Field");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return None;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(4),
                Constraint::Min(0),
            ])
            .split(inner);

        frame.render_widget(
            Paragraph::new(Text::from(stylized_body_lines(
                [
                    format!("Editing {label}"),
                    "Type a new value, then press `Enter` to save.".to_string(),
                    "`Esc` cancels.".to_string(),
                ],
                self.theme,
            )))
            .style(self.theme.panel_style())
            .wrap(Wrap { trim: false }),
            chunks[0],
        );

        draw_text_field(frame, chunks[1], label, buffer, self.theme)
    }

    fn draw_select_path_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        label: &str,
        buffer: &str,
        directories: &[String],
        selected: usize,
    ) -> Option<(u16, u16)> {
        let block = self.theme.block("Choose Folder");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return None;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Length(4),
                Constraint::Min(0),
            ])
            .split(inner);

        frame.render_widget(
            Paragraph::new(Text::from(stylized_body_lines(
                [
                    format!("Choosing {label}"),
                    "Type to filter directories. Relative input keeps relative suggestions."
                        .to_string(),
                    "`Tab`, `Right`, or `l` selects the highlighted folder.".to_string(),
                    "`Enter` saves the current path. `Esc` cancels.".to_string(),
                ],
                self.theme,
            )))
            .style(self.theme.panel_style())
            .wrap(Wrap { trim: false }),
            chunks[0],
        );

        let cursor = draw_text_field(frame, chunks[1], label, buffer, self.theme);
        let items = if directories.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No matching folders found.",
                muted_style(self.theme),
            )))]
        } else {
            directories
                .iter()
                .map(|directory| ListItem::new(directory.clone()))
                .collect::<Vec<_>>()
        };
        render_selectable_list(
            frame,
            chunks[2],
            self.theme.block("Folders"),
            items,
            (!directories.is_empty()).then_some(selected),
            self.theme,
        );

        cursor
    }
}

pub(super) fn draw_text_field(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    buffer: &str,
    theme: ThemePalette,
) -> Option<(u16, u16)> {
    let block = theme
        .block(title.to_string())
        .border_style(Style::default().fg(theme.focus_border).bg(theme.panel_bg));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return None;
    }

    let (display, cursor_offset) = input_window(buffer, inner.width as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(display, theme.input_style())))
            .style(theme.input_style()),
        inner,
    );

    Some((inner.x + cursor_offset as u16, inner.y))
}

fn input_window(buffer: &str, width: usize) -> (String, usize) {
    if width <= 1 {
        return (String::new(), 0);
    }

    let visible_len = width - 1;
    let total_len = buffer.chars().count();
    let start = total_len.saturating_sub(visible_len);
    let display = buffer.chars().skip(start).collect::<String>();
    let cursor_offset = display.chars().count().min(width - 1);

    (display, cursor_offset)
}
