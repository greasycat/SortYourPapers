use std::time::{Duration, Instant};

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Line, Modifier, Span, Style},
    widgets::{Gauge, Paragraph, Wrap},
};

use super::layout::draw_scrolled_panel;
use crate::{
    terminal,
    tui::{
        app::App,
        model::{OperationDetail, OperationState, OperationTab},
        taxonomy_tree::render_category_tree,
        theme::ThemePalette,
        ui_widgets::{muted_style, render_tabs, stylized_body_line},
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StageTimingSnapshot {
    pub(crate) stage: String,
    pub(crate) elapsed: Duration,
    pub(crate) running: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct StageTimingBar {
    pub(crate) stage: String,
    pub(crate) elapsed_label: String,
    pub(crate) ratio: f64,
}

impl StageTimingBar {
    fn style(&self, theme: ThemePalette) -> Style {
        Style::default().fg(theme.info).bg(theme.panel_bg)
    }
}

pub(crate) fn stage_timing_bars(timings: Vec<StageTimingSnapshot>) -> Vec<StageTimingBar> {
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

impl App {
    pub(super) fn draw_operation(&self, frame: &mut Frame, area: Rect) {
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

fn operation_status_height(area_height: u16, progress_count: usize) -> u16 {
    let preferred = if progress_count == 0 {
        6
    } else {
        progress_count as u16 + 5
    };
    let max_height = area_height.saturating_sub(6).max(4);
    preferred.clamp(4, max_height)
}

fn stage_label_width(row_width: u16) -> u16 {
    row_width.saturating_sub(10).clamp(12, 24)
}
