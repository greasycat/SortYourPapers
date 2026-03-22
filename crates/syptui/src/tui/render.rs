use std::time::{Duration, Instant};

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{Frame, Line, Modifier, Span, Style, Text},
    widgets::{Block, Clear, Gauge, ListItem, Paragraph, Wrap},
};

use crate::terminal;

use super::{
    app::App,
    model::{OperationDetail, OperationState, OperationTab, Overlay, Screen},
    session_view::rerun_stage_name,
    taxonomy_review::ReviewPane,
    taxonomy_tree::{render_category_tree, render_section_tree},
    theme::ThemePalette,
    ui_widgets::{
        muted_style, render_scrolled_paragraph, render_selectable_list, render_tabs,
        stylized_body_line, stylized_body_lines,
    },
};

const STACKED_SCREEN_WIDTH: u16 = 100;
const STACKED_WORKSPACE_WIDTH: u16 = 140;
const STACKED_REVIEW_WIDTH: u16 = 120;
const OVERLAY_PADDING: u16 = 1;

impl App {
    pub(super) fn draw(&self, frame: &mut Frame) {
        frame.render_widget(Block::default().style(self.theme.app_style()), frame.area());
        let header_height = header_height(frame.area().width, self.shortcut_actions());
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

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
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

    fn draw_home(&self, frame: &mut Frame, area: Rect) {
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

    fn draw_extract(&self, frame: &mut Frame, area: Rect) {
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

    fn draw_operation_status(&self, frame: &mut Frame, area: Rect) {
        let block = self.theme.block("Status");
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
                        .fg(self.theme.info)
                        .bg(self.theme.panel_bg)
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
                .style(self.theme.panel_style())
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
        let titles = OperationTab::ALL
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                Line::styled(
                    format!("{} {}", index + 1, tab.label(&self.operation.detail)),
                    muted_style(self.theme),
                )
            })
            .collect::<Vec<_>>();
        render_tabs(
            frame,
            area,
            self.theme.block("Views"),
            titles,
            self.operation.active_tab.index(),
            self.theme,
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
        let timing_bars = self.operation_stage_timing_bars();
        let run_summary_lines = self.operation_run_summary_lines();
        let top_height = (timing_bars.len() as u16 + 2)
            .max(6)
            .min(area.height.saturating_sub(6).max(6));
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(top_height), Constraint::Min(6)])
            .split(area);

        frame.render_widget(
            Paragraph::new(run_summary_lines)
                .style(self.theme.panel_style())
                .wrap(Wrap { trim: false })
                .block(self.theme.block("Run Summary")),
            chunks[1],
        );

        self.draw_operation_highlights(frame, chunks[0], &timing_bars);
    }

    fn draw_operation_highlights(
        &self,
        frame: &mut Frame,
        area: Rect,
        timing_bars: &[StageTimingBar],
    ) {
        let block = self.theme.block("Elasped Time");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        if timing_bars.is_empty() {
            frame.render_widget(
                Paragraph::new(vec![Line::from("No completed stage timings yet.")])
                    .style(self.theme.panel_style())
                    .wrap(Wrap { trim: false }),
                inner,
            );
            return;
        }

        let visible = usize::from(inner.height).min(timing_bars.len());
        let timing_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![Constraint::Length(1); visible])
            .split(inner);

        for (timing, row) in timing_bars
            .iter()
            .take(visible)
            .zip(timing_rows.iter().copied())
        {
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(stage_label_width(row.width)),
                    Constraint::Min(8),
                ])
                .split(row);

            frame.render_widget(Paragraph::new(timing.stage.clone()), columns[0]);
            frame.render_widget(
                Gauge::default()
                    .ratio(timing.ratio)
                    .label(timing.elapsed_label.clone())
                    .gauge_style(timing.style(self.theme))
                    .use_unicode(true),
                columns[1],
            );
        }
    }

    pub(super) fn operation_stage_timing_bars(&self) -> Vec<StageTimingBar> {
        stage_timing_bars(self.operation_stage_timing_snapshots())
    }

    fn operation_run_summary_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![stylized_body_line(&self.operation.summary, self.theme)];
        let report_lines = self.operation_report_summary_lines();
        if !report_lines.is_empty() {
            lines.push(Line::from(String::new()));
            lines.extend(
                report_lines
                    .into_iter()
                    .map(|line| stylized_body_line(&line, self.theme)),
            );
        }

        let guidance = match self.operation.state {
            OperationState::Running => vec![
                "Use `2` Logs for raw output and retries.".to_string(),
                format!(
                    "Use `3` {} or `4` Planned Actions as artifacts arrive.",
                    self.operation.detail.tab_label()
                ),
            ],
            OperationState::Success => vec![
                format!(
                    "Next actions: `3` {}, `4` Planned Actions, `s` Sessions.",
                    self.operation.detail.tab_label()
                ),
                "`Esc` returns to the screen that launched this operation.".to_string(),
            ],
            OperationState::Failure => vec![
                "Use `2` Logs for details and `s` Sessions for follow-up.".to_string(),
                "`Esc` returns after the operation becomes idle.".to_string(),
            ],
            OperationState::Idle => vec![
                "No active operation is running.".to_string(),
                "Launch a run from the Run Configuration screen.".to_string(),
            ],
        };

        if !lines.is_empty() {
            lines.push(Line::from(String::new()));
        }
        lines.extend(
            guidance
                .into_iter()
                .map(|line| stylized_body_line(&line, self.theme)),
        );
        lines
    }

    fn operation_stage_timing_snapshots(&self) -> Vec<StageTimingSnapshot> {
        let mut timings = self
            .operation
            .stage_timings
            .iter()
            .map(|timing| StageTimingSnapshot {
                stage: timing.stage.clone(),
                elapsed: timing.elapsed,
                running: false,
            })
            .collect::<Vec<_>>();

        if let (Some(started_at), false) = (
            self.operation.stage_started_at,
            self.operation.stage_label.is_empty(),
        ) {
            timings.push(StageTimingSnapshot {
                stage: self.operation.stage_label.clone(),
                elapsed: Instant::now().saturating_duration_since(started_at),
                running: true,
            });
        }

        timings
    }

    fn draw_operation_logs_tab(&self, frame: &mut Frame, area: Rect) {
        draw_scrolled_panel(
            frame,
            area,
            &format!("Logs ({})", self.logs.len()),
            self.operation_log_lines(),
            self.operation.log_scroll,
            "No logs yet. Output will appear here while the operation runs.",
            self.theme,
        );
    }

    fn draw_operation_taxonomy_tab(&self, frame: &mut Frame, area: Rect) {
        match &self.operation.detail {
            OperationDetail::Text {
                title,
                empty_message,
                ..
            } => draw_scrolled_panel(
                frame,
                area,
                title,
                self.operation_taxonomy_lines(),
                self.operation.taxonomy_scroll,
                empty_message,
                self.theme,
            ),
            OperationDetail::Tree(categories) => {
                let mut state = self.operation.taxonomy_tree_state.borrow_mut();
                render_category_tree(
                    frame,
                    area,
                    self.theme.block("Taxonomy"),
                    categories,
                    &mut state,
                    self.theme,
                );
            }
            OperationDetail::None => {
                if let Some(categories) = &self.last_category_tree {
                    let mut state = self.operation.taxonomy_tree_state.borrow_mut();
                    render_category_tree(
                        frame,
                        area,
                        self.theme.block("Taxonomy"),
                        categories,
                        &mut state,
                        self.theme,
                    );
                } else {
                    draw_scrolled_panel(
                        frame,
                        area,
                        "Taxonomy",
                        Vec::new(),
                        0,
                        "Taxonomy not available yet. It appears after taxonomy synthesis or review.",
                        self.theme,
                    );
                }
            }
        }
    }

    fn draw_operation_report_tab(&self, frame: &mut Frame, area: Rect) {
        draw_scrolled_panel(
            frame,
            area,
            "Planned Actions",
            self.operation_report_lines(),
            self.operation.report_scroll,
            "Planned actions are not available yet. They appear after plan generation.",
            self.theme,
        );
    }

    fn shortcut_actions(&self) -> &'static [(&'static str, &'static str)] {
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

    fn draw_overlay(&self, frame: &mut Frame, overlay: &Overlay) {
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
                    .and_then(|stage| {
                        crate::session::commands::describe_rerun_impact(config, *stage).ok()
                    })
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

fn draw_taxonomy_review_suggestion_panel(
    frame: &mut Frame,
    area: Rect,
    review: &super::taxonomy_review::TaxonomyReviewView,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StageTimingSnapshot {
    pub(super) stage: String,
    pub(super) elapsed: Duration,
    pub(super) running: bool,
}

#[derive(Clone, Debug)]
pub(super) struct StageTimingBar {
    pub(super) stage: String,
    pub(super) elapsed_label: String,
    pub(super) ratio: f64,
}

impl StageTimingBar {
    fn style(&self, theme: ThemePalette) -> Style {
        Style::default().fg(theme.info).bg(theme.panel_bg)
    }
}

pub(super) fn stage_timing_bars(timings: Vec<StageTimingSnapshot>) -> Vec<StageTimingBar> {
    let timings = timings
        .into_iter()
        .filter(|timing| timing.stage != "inspect-output")
        .collect::<Vec<_>>();
    let max_elapsed = timings
        .iter()
        .map(|timing| timing.elapsed)
        .max()
        .unwrap_or_default();
    let denominator = timing_progress_denominator(max_elapsed);

    timings
        .into_iter()
        .enumerate()
        .map(|(index, timing)| StageTimingBar {
            stage: format!("{}. {}", index + 1, timing.stage),
            elapsed_label: format!(
                "{}{}",
                terminal::format_duration(timing.elapsed),
                if timing.running { " (running)" } else { "" }
            ),
            ratio: timing_ratio(timing.elapsed, denominator),
        })
        .collect()
}

fn timing_progress_denominator(max_elapsed: Duration) -> Duration {
    if max_elapsed.is_zero() {
        Duration::from_millis(1)
    } else {
        max_elapsed.mul_f64(1.5)
    }
}

fn timing_ratio(elapsed: Duration, denominator: Duration) -> f64 {
    if denominator.is_zero() {
        0.0
    } else {
        (elapsed.as_secs_f64() / denominator.as_secs_f64()).clamp(0.0, 1.0)
    }
}

fn draw_text_field(
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

fn compact_overlay_rect(area: Rect, title: &str, lines: &[&str]) -> Rect {
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

fn overlay_block(title: &str, theme: ThemePalette) -> Block<'_> {
    theme.overlay_block(title.to_string(), OVERLAY_PADDING)
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

fn header_height(width: u16, actions: &[(&str, &str)]) -> u16 {
    let inner_width = usize::from(width.saturating_sub(2)).max(1);
    let hint_lines = wrapped_line_count_from_len(shortcut_hint_len(actions), inner_width);
    if hint_lines <= 1 {
        3
    } else {
        (hint_lines as u16 + 3).min(5)
    }
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

fn stage_label_width(row_width: u16) -> u16 {
    row_width.saturating_sub(10).clamp(12, 24)
}

fn draw_scrolled_panel(
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

fn draw_scrolled_panel_with_block(
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

fn focused_panel_block<'a>(title: &'a str, focused: bool, theme: ThemePalette) -> Block<'a> {
    theme.focused_block(title, focused)
}
