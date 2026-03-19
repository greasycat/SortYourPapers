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
                    let line = format_run_summary(index, run);
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
            vec![
                Line::from(format!("run_id: {}", run.run_id)),
                Line::from(format!("cwd: {}", run.cwd.display())),
                Line::from(format!("created_unix_ms: {}", run.created_unix_ms)),
                Line::from(format!(
                    "last stage: {}",
                    run.last_completed_stage
                        .map_or_else(|| "NotStarted".to_string(), |stage| format!("{stage:?}"))
                )),
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

fn format_run_summary(index: usize, run: &RunSummary) -> String {
    let stage = run
        .last_completed_stage
        .map_or_else(|| "NotStarted".to_string(), |stage| format!("{stage:?}"));
    let latest = if run.is_latest { " latest" } else { "" };
    format!(
        "{}. {} | stage={} | cwd={} | created_unix_ms={}{}",
        index + 1,
        run.run_id,
        stage,
        run.cwd.display(),
        run.created_unix_ms,
        latest
    )
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
