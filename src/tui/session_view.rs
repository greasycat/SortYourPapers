use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::{Local, TimeZone};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Style},
    widgets::{Block, Borders, ListItem, Paragraph, Wrap},
};

use crate::{
    papers::taxonomy::CategoryTree,
    session::{RunStage, RunSummary, RunWorkspace, SessionDetails, SessionStatusSummary},
    terminal::{Verbosity, report::render_report_lines},
};

use super::forms::bool_label;
use super::taxonomy_tree::{TaxonomyTreeState, render_category_tree, reset_state_for_categories};
use super::ui_widgets::{
    muted_style, render_scrolled_paragraph, render_selectable_list, render_tabs,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SessionFilter {
    #[default]
    All,
    Latest,
    Completed,
    Incomplete,
    Failed,
}

impl SessionFilter {
    const ALL: [Self; 5] = [
        Self::All,
        Self::Latest,
        Self::Completed,
        Self::Incomplete,
        Self::Failed,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Latest => "Latest",
            Self::Completed => "Completed",
            Self::Incomplete => "Incomplete",
            Self::Failed => "Failed",
        }
    }

    fn from_key(key: char) -> Option<Self> {
        match key {
            '1' => Some(Self::All),
            '2' => Some(Self::Latest),
            '3' => Some(Self::Completed),
            '4' => Some(Self::Incomplete),
            '5' => Some(Self::Failed),
            _ => None,
        }
    }

    fn matches(self, run: &RunSummary, status: &SessionStatusSummary) -> bool {
        match self {
            Self::All => true,
            Self::Latest => run.is_latest,
            Self::Completed => status.is_completed,
            Self::Incomplete => status.is_incomplete,
            Self::Failed => status.is_failed_looking,
        }
    }

    fn empty_message(self) -> &'static str {
        match self {
            Self::All => "No saved sessions found",
            Self::Latest => "No latest session is available",
            Self::Completed => "No completed sessions match this filter",
            Self::Incomplete => "No incomplete sessions match this filter",
            Self::Failed => "No failed-looking sessions match this filter",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::All => 0,
            Self::Latest => 1,
            Self::Completed => 2,
            Self::Incomplete => 3,
            Self::Failed => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SessionPreviewTab {
    #[default]
    Overview,
    Report,
    Taxonomy,
}

impl SessionPreviewTab {
    const ALL: [Self; 3] = [Self::Overview, Self::Report, Self::Taxonomy];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Report => "Report",
            Self::Taxonomy => "Taxonomy",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Overview => 0,
            Self::Report => 1,
            Self::Taxonomy => 2,
        }
    }

    fn from_index(index: usize) -> Self {
        Self::ALL[index.min(Self::ALL.len() - 1)]
    }
}

#[derive(Default)]
pub(super) struct SessionView {
    runs: Vec<RunSummary>,
    statuses: HashMap<String, SessionStatusSummary>,
    visible_runs: Vec<RunSummary>,
    selected: usize,
    filter: SessionFilter,
    preview_tab: SessionPreviewTab,
    preview_scroll: u16,
    selected_details: Option<SessionDetails>,
    selected_error: Option<String>,
}

impl SessionView {
    pub(super) fn refresh(&mut self) -> crate::error::Result<()> {
        let selected_run_id = self.selected_run_id();
        self.runs = RunWorkspace::list_runs()?;
        self.statuses = self
            .runs
            .iter()
            .map(|run| {
                let status =
                    RunWorkspace::inspect_run_status(run).unwrap_or_else(|_| fallback_status(run));
                (run.run_id.clone(), status)
            })
            .collect();
        self.apply_filter(selected_run_id.as_deref());
        Ok(())
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        if self.visible_runs.is_empty() {
            self.selected = 0;
            self.selected_details = None;
            self.selected_error = None;
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, self.visible_runs.len().saturating_sub(1) as isize) as usize;
        self.preview_scroll = 0;
        self.load_selected_details();
    }

    pub(super) fn set_filter_for_key(&mut self, key: char) {
        if let Some(filter) = SessionFilter::from_key(key) {
            self.filter = filter;
            self.apply_filter(self.selected_run_id().as_deref());
        }
    }

    pub(super) fn switch_preview_tab(&mut self, delta: i8) {
        let current = self.preview_tab.index();
        let target = if delta < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(SessionPreviewTab::ALL.len() - 1)
        };
        self.preview_tab = SessionPreviewTab::from_index(target);
        self.preview_scroll = 0;
    }

    pub(super) fn scroll_preview(&mut self, delta: isize) {
        self.preview_scroll =
            (self.preview_scroll as isize + delta).clamp(0, u16::MAX as isize) as u16;
    }

    pub(super) fn selected_run_id(&self) -> Option<String> {
        self.visible_runs
            .get(self.selected)
            .map(|run| run.run_id.clone())
    }

    pub(super) fn draw(&self, frame: &mut Frame, area: Rect) {
        let now_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis());
        let chunks = if area.width < 120 {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
                .split(area)
        };

        self.draw_list_column(frame, chunks[0], now_unix_ms);
        self.draw_detail_column(frame, chunks[1], now_unix_ms);
    }

    #[cfg(test)]
    pub(super) fn replace_runs_for_tests(&mut self, runs: Vec<RunSummary>) {
        self.runs = runs.clone();
        self.statuses = runs
            .iter()
            .map(|run| (run.run_id.clone(), fallback_status(run)))
            .collect();
        self.visible_runs = runs;
        self.selected = 0;
        self.selected_details = None;
        self.selected_error = None;
    }

    #[cfg(test)]
    pub(super) fn set_status_for_tests(&mut self, run_id: &str, status: SessionStatusSummary) {
        self.statuses.insert(run_id.to_string(), status);
        self.apply_filter(self.selected_run_id().as_deref());
    }

    #[cfg(test)]
    pub(super) fn set_selected_details_for_tests(&mut self, details: SessionDetails) {
        self.selected_details = Some(details);
        self.selected_error = None;
        self.preview_scroll = 0;
    }

    #[cfg(test)]
    pub(super) fn preview_scroll_for_tests(&self) -> u16 {
        self.preview_scroll
    }

    #[cfg(test)]
    pub(super) fn preview_tab_label_for_tests(&self) -> &'static str {
        self.preview_tab.label()
    }

    fn apply_filter(&mut self, preferred_run_id: Option<&str>) {
        self.visible_runs = self
            .runs
            .iter()
            .filter(|run| {
                self.statuses
                    .get(&run.run_id)
                    .is_some_and(|status| self.filter.matches(run, status))
            })
            .cloned()
            .collect();

        self.selected = preferred_run_id
            .and_then(|run_id| {
                self.visible_runs
                    .iter()
                    .position(|run| run.run_id == run_id)
            })
            .unwrap_or(0);
        if self.selected >= self.visible_runs.len() {
            self.selected = self.visible_runs.len().saturating_sub(1);
        }
        self.preview_scroll = 0;
        self.load_selected_details();
    }

    fn load_selected_details(&mut self) {
        self.selected_details = None;
        self.selected_error = None;
        let Some(run) = self.visible_runs.get(self.selected) else {
            return;
        };

        match RunWorkspace::inspect_run(run) {
            Ok(details) => {
                self.selected_details = Some(details);
            }
            Err(err) => self.selected_error = Some(err.to_string()),
        }
    }

    fn draw_list_column(&self, frame: &mut Frame, area: Rect, now_unix_ms: u128) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(area);

        let filter_titles = SessionFilter::ALL
            .iter()
            .enumerate()
            .map(|(index, filter)| {
                Line::styled(format!("{} {}", index + 1, filter.label()), muted_style())
            })
            .collect::<Vec<_>>();
        render_tabs(
            frame,
            chunks[0],
            Block::default().title("Filters").borders(Borders::ALL),
            filter_titles,
            self.filter.index(),
        );

        let items = if self.visible_runs.is_empty() {
            vec![ListItem::new(self.filter.empty_message())]
        } else {
            self.visible_runs
                .iter()
                .enumerate()
                .map(|(index, run)| {
                    let status = self
                        .statuses
                        .get(&run.run_id)
                        .cloned()
                        .unwrap_or_else(|| fallback_status(run));
                    let line = format_run_summary(index, run, &status, now_unix_ms);
                    ListItem::new(Line::styled(
                        line,
                        Style::default().fg(run_status_color(run, &status)),
                    ))
                })
                .collect::<Vec<_>>()
        };
        render_selectable_list(
            frame,
            chunks[1],
            Block::default().title("Saved Runs").borders(Borders::ALL),
            items,
            (!self.visible_runs.is_empty()).then_some(self.selected),
        );
    }

    fn draw_detail_column(&self, frame: &mut Frame, area: Rect, now_unix_ms: u128) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(12),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(area);

        let overview_lines = self.overview_lines(now_unix_ms);
        frame.render_widget(
            Paragraph::new(overview_lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Overview").borders(Borders::ALL)),
            chunks[0],
        );

        let tab_titles = SessionPreviewTab::ALL
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                Line::styled(format!("{} {}", index + 1, tab.label()), muted_style())
            })
            .collect::<Vec<_>>();
        render_tabs(
            frame,
            chunks[1],
            Block::default().title("Preview Tabs").borders(Borders::ALL),
            tab_titles,
            self.preview_tab.index(),
        );

        let preview_title = format!("Preview: {}", self.preview_tab.label());
        match self.preview_tab {
            SessionPreviewTab::Taxonomy => {
                if let Some(categories) = self.preview_taxonomy_categories() {
                    let mut tree_state = TaxonomyTreeState::default();
                    reset_state_for_categories(&mut tree_state, categories);
                    render_category_tree(
                        frame,
                        chunks[2],
                        Block::default().title(preview_title).borders(Borders::ALL),
                        categories,
                        &mut tree_state,
                    );
                } else {
                    render_scrolled_paragraph(
                        frame,
                        chunks[2],
                        Block::default().title(preview_title).borders(Borders::ALL),
                        vec![Line::from("No saved taxonomy exists for this session.")],
                        0,
                        true,
                    );
                }
            }
            _ => {
                let preview_lines = self.preview_lines();
                let preview_content = if preview_lines.is_empty() {
                    vec![Line::from(
                        "No preview is available for the selected session.",
                    )]
                } else {
                    preview_lines
                        .into_iter()
                        .map(Line::from)
                        .collect::<Vec<_>>()
                };
                render_scrolled_paragraph(
                    frame,
                    chunks[2],
                    Block::default().title(preview_title).borders(Borders::ALL),
                    preview_content,
                    self.preview_scroll,
                    true,
                );
            }
        }
    }

    fn overview_lines(&self, now_unix_ms: u128) -> Vec<Line<'static>> {
        let Some(run) = self.visible_runs.get(self.selected) else {
            return vec![Line::from("No run selected")];
        };
        let status = self
            .statuses
            .get(&run.run_id)
            .cloned()
            .unwrap_or_else(|| fallback_status(run));
        let relative_age = format_relative_age(run.created_unix_ms, now_unix_ms);
        let mut lines = vec![
            Line::from(format!("run_id: {}", run.run_id)),
            Line::from(format!("state: {}", run_status_label(run, &status))),
            Line::from(format!(
                "last stage: {}",
                run.last_completed_stage.map_or_else(
                    || "Not started".to_string(),
                    |stage| stage.description().to_string()
                )
            )),
            Line::from(format!(
                "started: {}",
                format_exact_time(run.created_unix_ms)
            )),
            Line::from(format!("age: {relative_age}")),
            Line::from(format!("latest: {}", bool_label(run.is_latest))),
        ];

        if let Some(details) = &self.selected_details {
            lines.push(Line::from(format!(
                "mode: {}",
                if details.config.dry_run {
                    "preview"
                } else {
                    "apply"
                }
            )));
            lines.push(Line::from(format!(
                "provider: {} / {}",
                details.config.llm_provider, details.config.llm_model
            )));
            lines.push(Line::from(format!(
                "report: {}  taxonomy: {}",
                bool_label(details.report.is_some()),
                bool_label(details.taxonomy.is_some())
            )));
        } else if let Some(error) = &self.selected_error {
            lines.push(Line::from(""));
            lines.push(Line::from(format!("inspection error: {error}")));
        }

        lines
    }

    fn preview_lines(&self) -> Vec<String> {
        if let Some(error) = &self.selected_error {
            return vec![format!("Could not load selected session details: {error}")];
        }

        let Some(details) = &self.selected_details else {
            return Vec::new();
        };

        match self.preview_tab {
            SessionPreviewTab::Overview => self.overview_preview_lines(details),
            SessionPreviewTab::Report => details
                .report
                .as_ref()
                .map(|report| render_report_lines(report, Verbosity::new(false, false, false)))
                .unwrap_or_else(|| vec!["No saved report exists for this session.".to_string()]),
            SessionPreviewTab::Taxonomy => Vec::new(),
        }
    }

    fn preview_taxonomy_categories(&self) -> Option<&[CategoryTree]> {
        self.selected_details
            .as_ref()
            .and_then(|details| details.taxonomy.as_deref())
    }

    fn overview_preview_lines(&self, details: &SessionDetails) -> Vec<String> {
        let mut lines = vec![
            format!(
                "status: {}",
                run_status_label(&details.run, &details.status)
            ),
            format!("workspace: {}", details.run.cwd.display()),
            format!(
                "saved mode: {}",
                if details.config.dry_run {
                    "preview"
                } else {
                    "apply"
                }
            ),
            String::new(),
            "Available stage artifacts:".to_string(),
        ];

        if details.available_stage_artifacts.is_empty() {
            lines.push("  none".to_string());
        } else {
            for stage in &details.available_stage_artifacts {
                lines.push(format!(
                    "  {} | {}",
                    rerun_stage_name(*stage),
                    stage.description()
                ));
            }
        }

        lines.extend([
            String::new(),
            "Actions:".to_string(),
            "  p resume preview".to_string(),
            "  a resume apply".to_string(),
            "  r rerun preview".to_string(),
            "  x rerun apply".to_string(),
            "  v review taxonomy".to_string(),
            "  d delete selected".to_string(),
            "  c clear incomplete".to_string(),
            "  C clear all".to_string(),
        ]);
        lines
    }
}

fn format_run_summary(
    index: usize,
    run: &RunSummary,
    status: &SessionStatusSummary,
    now_unix_ms: u128,
) -> String {
    let stage = run
        .last_completed_stage
        .map_or("not-started", rerun_stage_name);
    let latest = if run.is_latest { " | latest" } else { "" };
    format!(
        "{}. {} | {} | {} | {}{}",
        index + 1,
        run.run_id,
        run_status_label(run, status),
        stage,
        format_relative_age(run.created_unix_ms, now_unix_ms),
        latest
    )
}

fn fallback_status(run: &RunSummary) -> SessionStatusSummary {
    SessionStatusSummary {
        is_completed: run.last_completed_stage == Some(RunStage::Completed),
        is_incomplete: run.last_completed_stage != Some(RunStage::Completed),
        is_failed_looking: run.last_completed_stage != Some(RunStage::Completed),
    }
}

fn run_status_label(run: &RunSummary, status: &SessionStatusSummary) -> &'static str {
    if status.is_completed && status.is_failed_looking {
        "failed"
    } else if status.is_completed {
        "completed"
    } else if run.last_completed_stage.is_some() {
        "incomplete"
    } else {
        "new"
    }
}

fn run_status_color(run: &RunSummary, status: &SessionStatusSummary) -> Color {
    if status.is_completed && status.is_failed_looking {
        Color::Red
    } else if status.is_completed {
        Color::Green
    } else if run.last_completed_stage.is_some() {
        Color::Yellow
    } else {
        Color::Gray
    }
}

fn format_relative_age(created_unix_ms: u128, now_unix_ms: u128) -> String {
    let delta_ms = now_unix_ms.saturating_sub(created_unix_ms);
    let delta_secs = delta_ms / 1_000;

    match delta_secs {
        0..=44 => "just now".to_string(),
        45..=89 => "1m ago".to_string(),
        90..=3_599 => format!("{}m ago", delta_secs / 60),
        3_600..=5_399 => "1h ago".to_string(),
        5_400..=86_399 => format!("{}h ago", delta_secs / 3_600),
        86_400..=129_599 => "1d ago".to_string(),
        _ => format!("{}d ago", delta_secs / 86_400),
    }
}

fn format_exact_time(created_unix_ms: u128) -> String {
    let Ok(created_unix_ms) = i64::try_from(created_unix_ms) else {
        return "unknown".to_string();
    };

    Local
        .timestamp_millis_opt(created_unix_ms)
        .single()
        .map(|datetime| datetime.format("%Y-%m-%d %H:%M:%S %Z").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

pub(super) fn rerun_stage_name(stage: RunStage) -> &'static str {
    match stage {
        RunStage::DiscoverInput => "discover-input",
        RunStage::DiscoverOutput => "discover-output",
        RunStage::Dedupe => "dedupe",
        RunStage::FilterSize => "filter-size",
        RunStage::ExtractText => "extract-text",
        RunStage::BuildLlmClient => "build-llm-client",
        RunStage::ExtractKeywords => "extract-keywords",
        RunStage::SynthesizeCategories => "synthesize-categories",
        RunStage::InspectOutput => "inspect-output",
        RunStage::GeneratePlacements => "generate-placements",
        RunStage::BuildPlan => "build-plan",
        RunStage::ExecutePlan => "execute-plan",
        RunStage::Completed => "completed",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::session::{RunSummary, SessionConfigSummary, SessionDetails, SessionStatusSummary};

    use super::{
        SessionPreviewTab, format_exact_time, format_relative_age, format_run_summary,
        run_status_label,
    };
    use crate::report::RunReport;
    use crate::session::RunStage;

    fn sample_run(stage: Option<RunStage>) -> RunSummary {
        RunSummary {
            run_id: "run-123".to_string(),
            created_unix_ms: 60_000,
            cwd: PathBuf::from("/tmp/project"),
            last_completed_stage: stage,
            is_latest: true,
        }
    }

    fn sample_status(
        is_completed: bool,
        is_incomplete: bool,
        is_failed_looking: bool,
    ) -> SessionStatusSummary {
        SessionStatusSummary {
            is_completed,
            is_incomplete,
            is_failed_looking,
        }
    }

    #[test]
    fn relative_age_prefers_human_scannable_labels() {
        assert_eq!(format_relative_age(60_000, 60_500), "just now");
        assert_eq!(format_relative_age(0, 120_000), "2m ago");
        assert_eq!(format_relative_age(0, 7_200_000), "2h ago");
        assert_eq!(format_relative_age(0, 172_800_000), "2d ago");
    }

    #[test]
    fn run_status_label_matches_completion_state() {
        assert_eq!(
            run_status_label(&sample_run(None), &sample_status(false, true, true)),
            "new"
        );
        assert_eq!(
            run_status_label(
                &sample_run(Some(RunStage::ExtractText)),
                &sample_status(false, true, true)
            ),
            "incomplete"
        );
        assert_eq!(
            run_status_label(
                &sample_run(Some(RunStage::Completed)),
                &sample_status(true, false, false)
            ),
            "completed"
        );
        assert_eq!(
            run_status_label(
                &sample_run(Some(RunStage::Completed)),
                &sample_status(true, false, true)
            ),
            "failed"
        );
    }

    #[test]
    fn run_summary_uses_human_age_instead_of_raw_timestamp() {
        let line = format_run_summary(
            0,
            &sample_run(Some(RunStage::ExtractText)),
            &sample_status(false, true, true),
            180_000,
        );

        assert!(line.contains("incomplete"));
        assert!(line.contains("extract-text"));
        assert!(line.contains("2m ago"));
        assert!(!line.contains("created_unix_ms"));
    }

    #[test]
    fn exact_time_format_returns_displayable_timestamp() {
        assert_ne!(format_exact_time(60_000), "unknown");
    }

    #[test]
    fn overview_preview_tab_index_round_trip_is_stable() {
        assert_eq!(
            SessionPreviewTab::from_index(0),
            SessionPreviewTab::Overview
        );
        assert_eq!(
            SessionPreviewTab::from_index(9),
            SessionPreviewTab::Taxonomy
        );
    }

    #[test]
    fn sample_detail_construction_stays_compilable_for_parent_module_tests() {
        let detail = SessionDetails {
            run: sample_run(Some(RunStage::Completed)),
            config: SessionConfigSummary {
                dry_run: true,
                llm_provider: "gemini".to_string(),
                llm_model: "gemini-3-flash-preview".to_string(),
            },
            status: sample_status(true, false, false),
            report: Some(RunReport::new(true)),
            taxonomy: None,
            available_stage_artifacts: vec![RunStage::ExtractText],
        };

        assert_eq!(detail.available_stage_artifacts.len(), 1);
    }
}
