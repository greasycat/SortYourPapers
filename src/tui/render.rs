use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Modifier, Span, Style, Text},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap},
};

use super::{
    app::App,
    forms::HOME_ITEMS,
    model::{OperationState, OperationTab, Overlay, Screen},
    session_view::rerun_stage_name,
    taxonomy_review::ReviewPane,
};

impl App {
    pub(super) fn draw(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(10)])
            .split(frame.area());

        self.draw_header(frame, chunks[0]);
        match self.screen {
            Screen::Home => self.draw_home(frame, chunks[1]),
            Screen::RunForm => self.run_form.draw(frame, chunks[1]),
            Screen::Sessions => self.session_view.draw(frame, chunks[1]),
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

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let title = match self.screen {
            Screen::Home => "Home",
            Screen::RunForm => "Run Configuration",
            Screen::Sessions => "Sessions",
            Screen::Operation => &self.operation.title,
            Screen::TaxonomyReview => "Taxonomy Review",
        };
        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let left_width = (title.chars().count() + self.operation.state.label().chars().count() + 22)
            .min(inner.width.saturating_sub(1) as usize) as u16;
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(left_width), Constraint::Min(1)])
            .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
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
            ])),
            chunks[0],
        );
        frame.render_widget(
            Paragraph::new(Line::from(shortcut_chip_spans(self.shortcut_actions())))
                .alignment(Alignment::Right),
            chunks[1],
        );
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

    fn draw_taxonomy_review(&self, frame: &mut Frame, area: Rect) -> Option<(u16, u16)> {
        let Some(review) = &self.taxonomy_review else {
            return None;
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(5), Constraint::Min(0)])
            .split(area);

        frame.render_widget(
            Paragraph::new(
                review
                    .status_lines()
                    .into_iter()
                    .map(Line::from)
                    .collect::<Vec<_>>(),
            )
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title("Review Status")
                    .borders(Borders::ALL),
            ),
            chunks[0],
        );

        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[1]);
        let left = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8),
                Constraint::Min(8),
                Constraint::Length(8),
            ])
            .split(content[0]);

        let cursor = self.draw_taxonomy_review_suggestion_panel(frame, left[0]);
        draw_scrolled_panel_with_block(
            frame,
            left[1],
            focused_panel_block(
                "Suggested Taxonomy",
                review.focused_pane == ReviewPane::Candidate,
            ),
            review.candidate_lines(),
            review.candidate_scroll,
            "No candidate yet. Submit a suggestion to compare a proposed taxonomy.",
        );
        draw_scrolled_panel_with_block(
            frame,
            left[2],
            focused_panel_block("History", review.focused_pane == ReviewPane::History),
            review.history_lines(),
            review.history_scroll,
            "No suggestions submitted yet.",
        );
        draw_scrolled_panel_with_block(
            frame,
            content[1],
            focused_panel_block(
                "Accepted Taxonomy",
                review.focused_pane == ReviewPane::Accepted,
            ),
            review.accepted_lines(),
            review.accepted_scroll,
            "No accepted taxonomy is available.",
        );

        cursor
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

    fn shortcut_actions(&self) -> &'static [(&'static str, &'static str)] {
        match self.screen {
            Screen::Home => &[("↑/↓", "move"), ("Enter", "open"), ("Esc", "quit")],
            Screen::RunForm => &[
                ("↑/↓ or j/k", "move"),
                ("←/→ or h/l", "column"),
                ("Enter", "edit"),
                ("Space", "toggle"),
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
                ("Esc", "back"),
            ],
            Screen::Operation => &[
                ("Tab/h/l", "switch"),
                ("1-4", "jump tab"),
                ("j/k", "scroll"),
                ("PgUp/PgDn", "page"),
                ("g/G", "start/end"),
                ("s", "sessions"),
                ("Esc", "back when idle"),
            ],
            Screen::TaxonomyReview => self
                .taxonomy_review
                .as_ref()
                .map(|review| review.shortcut_actions())
                .unwrap_or(&[("Tab/h/l", "change pane"), ("j/k", "scroll")]),
        }
    }

    fn draw_overlay(&self, frame: &mut Frame, overlay: &Overlay) {
        let area = match overlay {
            Overlay::EditField { .. } | Overlay::SelectRerunStage { .. } => {
                centered_rect(70, 60, frame.area())
            }
            Overlay::Confirm { title, message, .. } => compact_overlay_rect(
                frame.area(),
                title,
                &[
                    message.as_str(),
                    "",
                    "Enter or y confirm",
                    "Esc cancel",
                ],
            ),
            Overlay::Notice { title, message } => compact_overlay_rect(
                frame.area(),
                title,
                &[message.as_str(), "", "Enter or Esc dismiss"],
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
                stages,
                selected,
                config,
                ..
            } => {
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
                    .split(area);

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
                    chunks[0],
                );

                let impact_lines = stages
                    .get(*selected)
                    .and_then(|stage| {
                        crate::session::commands::describe_rerun_impact(config, *stage).ok()
                    })
                    .map(|impact| impact.lines())
                    .unwrap_or_else(|| vec!["Could not describe rerun impact.".to_string()]);
                frame.render_widget(
                    Paragraph::new(impact_lines.into_iter().map(Line::from).collect::<Vec<_>>())
                        .wrap(Wrap { trim: false })
                        .block(
                            Block::default()
                                .title("Rerun Consequences")
                                .borders(Borders::ALL),
                        ),
                    chunks[1],
                );
            }
        }
    }

    fn draw_taxonomy_review_suggestion_panel(
        &self,
        frame: &mut Frame,
        area: Rect,
    ) -> Option<(u16, u16)> {
        let Some(review) = &self.taxonomy_review else {
            return None;
        };

        let block =
            focused_panel_block("Suggestion", review.focused_pane == ReviewPane::Suggestion);
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
            Paragraph::new(
                review
                    .suggestion_lines()
                    .into_iter()
                    .map(Line::from)
                    .collect::<Vec<_>>(),
            )
            .wrap(Wrap { trim: false })
            .scroll((review.focused_scroll().unwrap_or(0), 0)),
            chunks[0],
        );

        if review.editing && chunks.len() > 1 {
            return draw_text_field(frame, chunks[1], "Draft", &review.suggestion_buffer);
        }

        None
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

fn shortcut_chip_spans(actions: &[(&str, &str)]) -> Vec<Span<'static>> {
    let palette = [
        Color::LightCyan,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightMagenta,
        Color::LightBlue,
        Color::LightRed,
    ];

    let mut spans = Vec::with_capacity(actions.len() * 2);
    for (index, (key, action)) in actions.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }

        spans.push(Span::styled(
            format!(" {key}: {action} "),
            Style::default()
                .fg(Color::Black)
                .bg(palette[index % palette.len()])
                .add_modifier(Modifier::BOLD),
        ));
    }

    spans
}


fn compact_overlay_rect(area: Rect, title: &str, lines: &[&str]) -> Rect {
    let max_width = area.width.saturating_sub(4).max(1);
    let content_width = title
        .chars()
        .count()
        .max(lines.iter().map(|line| line.chars().count()).max().unwrap_or(0));
    let desired_width = (content_width + 4).clamp(28, max_width as usize) as u16;
    let inner_width = desired_width.saturating_sub(2).max(1) as usize;
    let wrapped_height = lines
        .iter()
        .map(|line| wrapped_line_count(line, inner_width))
        .sum::<usize>();
    let desired_height = (wrapped_height + 2).clamp(5, area.height.saturating_sub(2).max(5) as usize)
        as u16;

    centered_rect_exact(desired_width, desired_height, area)
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

fn draw_scrolled_panel_with_block(
    frame: &mut Frame,
    area: Rect,
    block: Block<'_>,
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
            .block(block),
        area,
    );
}

fn focused_panel_block<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let border_style = if focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style)
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
