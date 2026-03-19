use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Local, TimeZone};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Modifier, Span, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::{
    error::Result,
    session::{RunStage, RunSummary, RunWorkspace},
};

use super::forms::bool_label;

#[derive(Default)]
pub(super) struct SessionView {
    runs: Vec<RunSummary>,
    selected: usize,
}

impl SessionView {
    pub(super) fn refresh(&mut self) -> Result<()> {
        self.runs = RunWorkspace::list_runs()?;
        if self.selected >= self.runs.len() {
            self.selected = self.runs.len().saturating_sub(1);
        }
        Ok(())
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        if self.runs.is_empty() {
            self.selected = 0;
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, self.runs.len().saturating_sub(1) as isize) as usize;
    }

    pub(super) fn selected_run_id(&self) -> Option<String> {
        self.runs.get(self.selected).map(|run| run.run_id.clone())
    }

    pub(super) fn draw(&self, frame: &mut Frame, area: Rect) {
        let now_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis());
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);

        let lines = if self.runs.is_empty() {
            vec![Line::from("No saved sessions found")]
        } else {
            self.runs
                .iter()
                .enumerate()
                .map(|(index, run)| {
                    let line = format_run_summary(index, run, now_unix_ms);
                    if index == self.selected {
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
                .collect::<Vec<_>>()
        };
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Saved Runs").borders(Borders::ALL)),
            chunks[0],
        );

        let detail_lines = if let Some(run) = self.runs.get(self.selected) {
            let relative_age = format_relative_age(run.created_unix_ms, now_unix_ms);
            vec![
                Line::from(format!("run_id: {}", run.run_id)),
                Line::from(format!("cwd: {}", run.cwd.display())),
                Line::from(format!("state: {}", run_state_label(run))),
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
                Line::from(""),
                Line::from("p resume preview"),
                Line::from("a resume apply"),
                Line::from("r rerun preview"),
                Line::from("x rerun apply"),
                Line::from("v review taxonomy"),
                Line::from("d delete selected"),
                Line::from("c clear incomplete"),
            ]
        } else {
            vec![Line::from("No run selected")]
        };
        frame.render_widget(
            Paragraph::new(detail_lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().title("Details").borders(Borders::ALL)),
            chunks[1],
        );
    }
}

fn format_run_summary(index: usize, run: &RunSummary, now_unix_ms: u128) -> String {
    let stage = run
        .last_completed_stage
        .map_or("not-started", rerun_stage_name);
    let latest = if run.is_latest { " | latest" } else { "" };
    format!(
        "{}. {} | {} | {} | {}{}",
        index + 1,
        run.run_id,
        run_state_label(run),
        stage,
        format_relative_age(run.created_unix_ms, now_unix_ms),
        latest
    )
}

fn run_state_label(run: &RunSummary) -> &'static str {
    match run.last_completed_stage {
        Some(RunStage::Completed) => "completed",
        Some(_) => "resumable",
        None => "new",
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

    use crate::session::RunSummary;

    use super::{format_exact_time, format_relative_age, format_run_summary, run_state_label};
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

    #[test]
    fn relative_age_prefers_human_scannable_labels() {
        assert_eq!(format_relative_age(60_000, 60_500), "just now");
        assert_eq!(format_relative_age(0, 120_000), "2m ago");
        assert_eq!(format_relative_age(0, 7_200_000), "2h ago");
        assert_eq!(format_relative_age(0, 172_800_000), "2d ago");
    }

    #[test]
    fn run_state_label_matches_completion_state() {
        assert_eq!(run_state_label(&sample_run(None)), "new");
        assert_eq!(
            run_state_label(&sample_run(Some(RunStage::ExtractText))),
            "resumable"
        );
        assert_eq!(
            run_state_label(&sample_run(Some(RunStage::Completed))),
            "completed"
        );
    }

    #[test]
    fn run_summary_uses_human_age_instead_of_raw_timestamp() {
        let line = format_run_summary(0, &sample_run(Some(RunStage::ExtractText)), 180_000);

        assert!(line.contains("resumable"));
        assert!(line.contains("extract-text"));
        assert!(line.contains("2m ago"));
        assert!(!line.contains("created_unix_ms"));
    }

    #[test]
    fn exact_time_format_returns_displayable_timestamp() {
        assert_ne!(format_exact_time(60_000), "unknown");
    }
}
