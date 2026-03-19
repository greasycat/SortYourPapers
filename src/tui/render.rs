use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Modifier, Span, Style, Text},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap},
};

use crate::terminal;

use super::{
    app::App,
    forms::HOME_ITEMS,
    model::{OperationDetail, Overlay, Screen},
    session_view::rerun_stage_name,
};

impl App {
    pub(super) fn draw(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(11),
            ])
            .split(frame.area());

        self.draw_header(frame, chunks[0]);
        match self.screen {
            Screen::Home => self.draw_home(frame, chunks[1]),
            Screen::RunForm => self.run_form.draw(frame, chunks[1]),
            Screen::Sessions => self.session_view.draw(frame, chunks[1]),
            Screen::Operation => self.draw_operation(frame, chunks[1]),
        }
        self.draw_footer(frame, chunks[2]);

        if let Some(overlay) = &self.overlay {
            self.draw_overlay(frame, overlay);
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let title = match self.screen {
            Screen::Home => "Home",
            Screen::RunForm => "Run Configuration",
            Screen::Sessions => "Sessions",
            Screen::Operation => &self.operation.title,
        };
        let status = if self.operation.running {
            "busy"
        } else if self.operation.success {
            "ready"
        } else {
            "idle"
        };
        let header = Paragraph::new(Line::from(vec![
            Span::styled(
                " SortYourPapers ",
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::raw(format!(" {title}")),
            Span::raw(" "),
            Span::styled(
                format!("[{status}]"),
                Style::default().fg(if self.operation.running {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            ),
        ]))
        .block(Block::default().borders(Borders::ALL));
        frame.render_widget(header, area);
    }

    fn draw_home(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(area);

        let menu_lines = HOME_ITEMS
            .iter()
            .enumerate()
            .map(|(index, item)| {
                if index == self.home_index {
                    Line::from(Span::styled(
                        format!("> {item}"),
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(format!("  {item}"))
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(menu_lines)
                .block(Block::default().title("Actions").borders(Borders::ALL)),
            chunks[0],
        );

        let help = Paragraph::new(Text::from(vec![
            Line::from("`syp` is the interactive terminal frontend."),
            Line::from(""),
            Line::from("Run: configure the full sorting workflow."),
            Line::from("Sessions: resume, rerun, review, remove, or clear saved runs."),
            Line::from("Quit: exit after confirmation."),
            Line::from(""),
            Line::from("Keys: ↑/↓ move, Enter open, Esc quit."),
        ]))
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Overview").borders(Borders::ALL));
        frame.render_widget(help, chunks[1]);
    }

    fn draw_operation(&self, frame: &mut Frame, area: Rect) {
        let status_height = operation_status_height(area.height, self.progress.len());
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(status_height),
                Constraint::Percentage(45),
                Constraint::Percentage(55),
            ])
            .split(area);

        self.draw_operation_status(frame, chunks[0]);

        let detail_lines = self.operation_detail_lines();
        frame.render_widget(
            Paragraph::new(detail_lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Details").borders(Borders::ALL)),
            chunks[1],
        );

        let log_lines = self
            .logs
            .iter()
            .rev()
            .take(18)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| Line::from(line.clone()))
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(log_lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Logs").borders(Borders::ALL)),
            chunks[2],
        );
    }

    fn operation_detail_lines(&self) -> Text<'static> {
        if let OperationDetail::Tree(categories) = &self.operation.detail {
            return Text::from(
                crate::terminal::report::render_category_tree(categories)
                    .lines()
                    .map(|line| Line::from(line.to_string()))
                    .collect::<Vec<_>>(),
            );
        }

        if let Some(report) = &self.last_report {
            return Text::from(
                crate::terminal::report::render_report_lines(
                    report,
                    terminal::Verbosity::new(false, false, false),
                )
                .into_iter()
                .map(Line::from)
                .collect::<Vec<_>>(),
            );
        }

        if let Some(tree) = &self.last_category_tree {
            return Text::from(
                tree.lines()
                    .map(|line| Line::from(line.to_string()))
                    .collect::<Vec<_>>(),
            );
        }

        Text::from(vec![Line::from(self.operation.summary.clone())])
    }

    fn draw_operation_status(&self, frame: &mut Frame, area: Rect) {
        if self.progress.is_empty() {
            frame.render_widget(
                Paragraph::new(vec![Line::from(self.operation.summary.clone())])
                    .wrap(Wrap { trim: false })
                    .block(Block::default().title("Status").borders(Borders::ALL)),
                area,
            );
            return;
        }

        let block = Block::default().title("Status").borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let visible = usize::from(inner.height).min(self.progress.len());
        let progress_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![Constraint::Length(1); visible])
            .split(inner);

        for (progress, chunk) in self
            .progress
            .iter()
            .take(visible)
            .zip(progress_chunks.iter().copied())
        {
            frame.render_widget(
                Gauge::default()
                    .ratio(progress.ratio())
                    .label(progress.label())
                    .gauge_style(progress.gauge_style())
                    .use_unicode(true),
                chunk,
            );
        }
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let help = match self.screen {
            Screen::Home => "↑/↓ move  Enter open  Esc quit",
            Screen::RunForm => {
                "↑/↓ or j/k move  ←/→ or h/l column  Enter edit/run  space toggle  Esc back"
            }
            Screen::Sessions => {
                "↑/↓ select  p preview  a apply  r rerun  x rerun-apply  v review  d delete  c clear  g refresh  Esc back"
            }
            Screen::Operation => "Esc back when idle",
        };
        frame.render_widget(
            Paragraph::new(help).block(Block::default().borders(Borders::ALL)),
            area,
        );
    }

    fn draw_overlay(&self, frame: &mut Frame, overlay: &Overlay) {
        let area = centered_rect(70, 60, frame.area());
        frame.render_widget(Clear, area);

        match overlay {
            Overlay::EditField { label, buffer } => {
                if let Some((x, y)) = self.draw_edit_field_overlay(frame, area, label, buffer) {
                    frame.set_cursor_position((x, y));
                }
            }
            Overlay::InspectPrompt {
                categories, input, ..
            } => {
                if let Some((x, y)) =
                    self.draw_inspect_prompt_overlay(frame, area, categories, input)
                {
                    frame.set_cursor_position((x, y));
                }
            }
            Overlay::ContinuePrompt { .. } => {
                let widget = Paragraph::new(Text::from(vec![
                    Line::from("Continue improving this taxonomy?"),
                    Line::from(""),
                    Line::from("y continue"),
                    Line::from("Enter or n finish"),
                    Line::from("Esc cancel"),
                ]))
                .block(
                    Block::default()
                        .title("Continue Improving")
                        .borders(Borders::ALL),
                );
                frame.render_widget(widget, area);
            }
            Overlay::Confirm { title, message, .. } => {
                let widget = Paragraph::new(Text::from(vec![
                    Line::from(message.clone()),
                    Line::from(""),
                    Line::from("Enter or y confirm"),
                    Line::from("Esc cancel"),
                ]))
                .wrap(Wrap { trim: false })
                .block(Block::default().title(title.clone()).borders(Borders::ALL));
                frame.render_widget(widget, area);
            }
            Overlay::SelectRerunStage {
                stages, selected, ..
            } => {
                let lines = stages
                    .iter()
                    .enumerate()
                    .map(|(index, stage)| {
                        let line = format!("{} {}", rerun_stage_name(*stage), stage.description());
                        if index == *selected {
                            Line::from(Span::styled(
                                format!("> {line}"),
                                Style::default()
                                    .fg(Color::Black)
                                    .bg(Color::Green)
                                    .add_modifier(Modifier::BOLD),
                            ))
                        } else {
                            Line::from(format!("  {line}"))
                        }
                    })
                    .collect::<Vec<_>>();
                frame.render_widget(
                    Paragraph::new(lines).wrap(Wrap { trim: false }).block(
                        Block::default()
                            .title("Select Rerun Stage")
                            .borders(Borders::ALL),
                    ),
                    area,
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
        let block = Block::default().title("Edit Field").borders(Borders::ALL);
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
            Paragraph::new(Text::from(vec![
                Line::from(format!("Editing {label}")),
                Line::from("Type a new value, then press Enter to save."),
                Line::from("Esc cancels."),
            ]))
            .wrap(Wrap { trim: false }),
            chunks[0],
        );

        draw_text_field(frame, chunks[1], label, buffer)
    }

    fn draw_inspect_prompt_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        categories: &[crate::papers::taxonomy::CategoryTree],
        input: &str,
    ) -> Option<(u16, u16)> {
        let block = Block::default()
            .title("Inspect Taxonomy")
            .borders(Borders::ALL);
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
            Paragraph::new(Text::from(vec![
                Line::from("Review the current taxonomy."),
                Line::from(""),
                Line::from("Type below, press Enter on an empty field to accept, or Esc to cancel."),
            ]))
            .wrap(Wrap { trim: false }),
            chunks[0],
        );

        let cursor = draw_text_field(frame, chunks[1], "Suggestion", input);

        let tree = crate::terminal::report::render_category_tree(categories);
        frame.render_widget(
            Paragraph::new(Text::from(vec![Line::from(""), Line::from(tree)]))
                .wrap(Wrap { trim: false }),
            chunks[2],
        );

        cursor
    }
}

fn draw_text_field(frame: &mut Frame, area: Rect, title: &str, buffer: &str) -> Option<(u16, u16)> {
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return None;
    }

    let (display, cursor_offset) = input_window(buffer, inner.width as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            display,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ))),
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

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
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

fn operation_status_height(area_height: u16, progress_count: usize) -> u16 {
    let preferred = if progress_count == 0 {
        5
    } else {
        progress_count as u16 + 2
    };
    let max_height = area_height.saturating_sub(6).max(3);
    preferred.clamp(3, max_height)
}
