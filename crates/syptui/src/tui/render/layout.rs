use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::Frame,
    widgets::Block,
};

use crate::tui::{
    theme::ThemePalette,
    ui_widgets::{render_scrolled_paragraph, stylized_body_line, stylized_body_lines},
};

const OVERLAY_PADDING: u16 = 1;

pub(super) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

pub(super) fn compact_overlay_rect(area: Rect, title: &str, lines: &[&str]) -> Rect {
    let max_width = area.width.saturating_sub(4).max(1);
    let content_width = title.chars().count().max(
        lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0),
    );
    let desired_width = (content_width + 4).clamp(28, max_width as usize) as u16;
    let inner_width = desired_width.saturating_sub(2 + OVERLAY_PADDING * 2).max(1) as usize;
    let wrapped_height = lines
        .iter()
        .map(|line| wrapped_line_count(line, inner_width))
        .sum::<usize>();
    let desired_height = (wrapped_height + 2 + usize::from(OVERLAY_PADDING) * 2)
        .clamp(5, area.height.saturating_sub(2).max(5) as usize) as u16;

    centered_rect_exact(desired_width, desired_height, area)
}

pub(super) fn overlay_block(title: &str, theme: ThemePalette) -> Block<'_> {
    theme.overlay_block(title.to_string(), OVERLAY_PADDING)
}

pub(super) fn draw_scrolled_panel(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    lines: Vec<String>,
    scroll: u16,
    empty_message: &str,
    theme: ThemePalette,
) {
    let content = if lines.is_empty() {
        vec![stylized_body_line(empty_message, theme)]
    } else {
        stylized_body_lines(lines, theme)
    };
    render_scrolled_paragraph(
        frame,
        area,
        theme.block(title.to_string()),
        content,
        scroll,
        true,
        theme,
    );
}

pub(super) fn draw_scrolled_panel_with_block(
    frame: &mut Frame,
    area: Rect,
    block: Block<'_>,
    lines: Vec<String>,
    scroll: u16,
    empty_message: &str,
    theme: ThemePalette,
) {
    let content = if lines.is_empty() {
        vec![stylized_body_line(empty_message, theme)]
    } else {
        stylized_body_lines(lines, theme)
    };
    render_scrolled_paragraph(frame, area, block, content, scroll, true, theme);
}

pub(super) fn focused_panel_block<'a>(
    title: &'a str,
    focused: bool,
    theme: ThemePalette,
) -> Block<'a> {
    theme.focused_block(title, focused)
}

fn wrapped_line_count(line: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }

    let len = line.chars().count();
    len.max(1).div_ceil(width)
}

fn centered_rect_exact(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}
