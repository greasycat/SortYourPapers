use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Modifier, Span, Style, Text},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap},
};

use super::{
    app::App,
    forms::HOME_ITEMS,
    model::{OperationState, OperationTab, Overlay, Screen},
    session_view::rerun_stage_name,
};

impl App {
    pub(super) fn draw(&self, frame: &mut Frame) {
        let footer_height = match self.screen {
            Screen::Operation => 3,
            _ => 11,
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(footer_height),
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
        let header = Paragraph::new(Line::from(vec![
            Span::styled(
                " SortYourPapers ",
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::raw(format!(" {title}")),
            Span::raw(" "),
            Span::styled(
                format!("[{}]", self.operation.state.label()),
                Style::default()
                    .fg(self.operation.state.color())
                    .add_modifier(Modifier::BOLD),
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
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(area);

        self.draw_operation_status(frame, chunks[0]);
        self.draw_operation_tabs(frame, chunks[1]);
        self.draw_operation_content(frame, chunks[2]);
    }

    fn draw_operation_status(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default().title("Status").borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let top_height = inner.height.min(3);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(top_height), Constraint::Min(0)])
            .split(inner);

        let stage_label = if self.operation.stage_label.is_empty() {
            "waiting".to_string()
        } else {
            self.operation.stage_label.clone()
        };
        let stage_message = if self.operation.stage_message.is_empty() {
            self.operation.summary.clone()
        } else {
            self.operation.stage_message.clone()
        };
        let summary_lines = vec![
            Line::from(vec![
                Span::styled(
                    "stage ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(stage_label),
            ]),
            Line::from(stage_message),
            Line::from(format!(
                "progress {}  alerts {}",
                self.progress.len(),
                self.operation.alerts.len()
            )),
        ];
        frame.render_widget(
            Paragraph::new(summary_lines)
                .wrap(Wrap { trim: false })
                .scroll((0, 0)),
            chunks[0],
        );

        if self.progress.is_empty() || chunks[1].height == 0 {
            return;
        }

        let visible = usize::from(chunks[1].height).min(self.progress.len());
        let progress_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![Constraint::Length(1); visible])
            .split(chunks[1]);

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

    fn draw_operation_tabs(&self, frame: &mut Frame, area: Rect) {
        let spans = OperationTab::ALL
            .iter()
            .enumerate()
            .flat_map(|(index, tab)| {
                let style = if *tab == self.operation.active_tab {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };
                vec![
                    Span::styled(format!(" {} {} ", index + 1, tab.label()), style),
                    Span::raw(" "),
                ]
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(Line::from(spans))
                .block(Block::default().title("Views").borders(Borders::ALL)),
            area,
        );
    }

    fn draw_operation_content(&self, frame: &mut Frame, area: Rect) {
        match self.operation.active_tab {
            OperationTab::Summary => self.draw_operation_summary_tab(frame, area),
            OperationTab::Logs => self.draw_operation_logs_tab(frame, area),
            OperationTab::Taxonomy => self.draw_operation_taxonomy_tab(frame, area),
            OperationTab::Report => self.draw_operation_report_tab(frame, area),
        }
    }

    fn draw_operation_summary_tab(&self, frame: &mut Frame, area: Rect) {
        let alerts_visible = self.operation.alerts.len().min(5);
        let alert_height =
            (alerts_visible as u16 + 2).clamp(4, area.height.saturating_sub(8).max(4));
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(6),
                Constraint::Length(alert_height),
                Constraint::Min(6),
            ])
            .split(area);

        let progress_summary = if self.progress.is_empty() {
            "none".to_string()
        } else {
            self.progress
                .iter()
                .take(4)
                .map(|entry| entry.label())
                .collect::<Vec<_>>()
                .join(" | ")
        };
        let highlight_lines = vec![
            Line::from(format!("state: {}", self.operation.state.label())),
            Line::from(format!("summary: {}", self.operation.summary)),
            Line::from(format!(
                "stage: {}",
                if self.operation.stage_label.is_empty() {
                    "waiting".to_string()
                } else {
                    self.operation.stage_label.clone()
                }
            )),
            Line::from(format!(
                "stage detail: {}",
                if self.operation.stage_message.is_empty() {
                    "No live stage detail yet.".to_string()
                } else {
                    self.operation.stage_message.clone()
                }
            )),
            Line::from(format!("active batches: {progress_summary}")),
            Line::from(format!(
                "artifacts: taxonomy={} report={}",
                yes_no(!self.operation_taxonomy_lines().is_empty()),
                yes_no(!self.operation_report_lines().is_empty())
            )),
        ];
        frame.render_widget(
            Paragraph::new(highlight_lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Highlights").borders(Borders::ALL)),
            chunks[0],
        );

        let alert_lines = if self.operation.alerts.is_empty() {
            vec![Line::from("No pinned warnings or errors yet.")]
        } else {
            self.operation
                .alerts
                .iter()
                .rev()
                .take(5)
                .rev()
                .map(|alert| {
                    Line::from(Span::styled(
                        alert.line(),
                        Style::default().fg(alert.color()),
                    ))
                })
                .collect::<Vec<_>>()
        };
        frame.render_widget(
            Paragraph::new(alert_lines)
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title("Pinned Alerts")
                        .borders(Borders::ALL),
                ),
            chunks[1],
        );

        let (title, body) = match self.operation.state {
            OperationState::Running => (
                "Live Run",
                vec![
                    Line::from("Use 2 Logs for raw output and retries."),
                    Line::from("Use 3 Taxonomy after category data appears."),
                    Line::from("Use 4 Report after a run report is emitted."),
                    Line::from("Esc returns after the run becomes idle."),
                ],
            ),
            OperationState::Success => (
                "Success",
                vec![
                    Line::from(self.operation.summary.clone()),
                    Line::from("Next actions: 3 Taxonomy, 4 Report, s Sessions."),
                    Line::from("Esc returns to the screen that launched this operation."),
                ],
            ),
            OperationState::Failure => (
                "Failure",
                vec![
                    Line::from(self.operation.summary.clone()),
                    Line::from("Review pinned alerts and the Logs tab first."),
                    Line::from("Next actions: 2 Logs, s Sessions, Esc back."),
                ],
            ),
            OperationState::Idle => (
                "Ready",
                vec![
                    Line::from(self.operation.summary.clone()),
                    Line::from("No active operation is running."),
                ],
            ),
        };
        frame.render_widget(
            Paragraph::new(body)
                .wrap(Wrap { trim: false })
                .block(Block::default().title(title).borders(Borders::ALL)),
            chunks[2],
        );
    }

    fn draw_operation_logs_tab(&self, frame: &mut Frame, area: Rect) {
        draw_scrolled_panel(
            frame,
            area,
            &format!("Logs ({})", self.logs.len()),
            self.operation_log_lines(),
            self.operation.log_scroll,
            "No logs yet. Output will appear here while the operation runs.",
        );
    }

    fn draw_operation_taxonomy_tab(&self, frame: &mut Frame, area: Rect) {
        draw_scrolled_panel(
            frame,
            area,
            "Taxonomy",
            self.operation_taxonomy_lines(),
            self.operation.taxonomy_scroll,
            "Taxonomy not available yet. It appears after taxonomy synthesis or review.",
        );
    }

    fn draw_operation_report_tab(&self, frame: &mut Frame, area: Rect) {
        draw_scrolled_panel(
            frame,
            area,
            "Report",
            self.operation_report_lines(),
            self.operation.report_scroll,
            "Report not available yet. It appears after report data is emitted.",
        );
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
            Screen::Operation => {
                "Tab/h/l switch  1-4 jump tab  j/k scroll  PgUp/PgDn page  g/G start/end  s sessions  Esc back when idle"
            }
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
            Overlay::Notice { title, message } => {
                let widget = Paragraph::new(Text::from(vec![
                    Line::from(message.clone()),
                    Line::from(""),
                    Line::from("Enter or Esc dismiss"),
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
                Line::from(
                    "Type below, press Enter on an empty field to accept, or Esc to cancel.",
                ),
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
        6
    } else {
        progress_count as u16 + 5
    };
    let max_height = area_height.saturating_sub(6).max(4);
    preferred.clamp(4, max_height)
}

fn draw_scrolled_panel(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    lines: Vec<String>,
    scroll: u16,
    empty_message: &str,
) {
    let content = if lines.is_empty() {
        vec![Line::from(empty_message.to_string())]
    } else {
        lines.into_iter().map(Line::from).collect::<Vec<_>>()
    };
    frame.render_widget(
        Paragraph::new(content)
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false })
            .block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
