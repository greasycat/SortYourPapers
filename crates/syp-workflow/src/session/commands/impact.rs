use crate::{
    config::AppConfig,
    error::{AppError, Result},
    llm::LlmUsageSummary,
    report::RunReport,
    session::{RunStage, stage_sequence},
};

pub(crate) const KEYWORD_BATCH_PROGRESS_FILE: &str = "06-extract-keywords-partial-batches.json";
pub(crate) const TAXONOMY_BATCH_PROGRESS_FILE: &str =
    "07-synthesize-categories-partial-batches.json";
pub(crate) const PLACEMENT_BATCH_PROGRESS_FILE: &str =
    "09-generate-placements-partial-batches.json";
pub(crate) const PLACEMENT_EVIDENCE_FILE: &str = "09-placement-evidence.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RerunArtifact {
    KeywordBatchProgress,
    TaxonomyBatchProgress,
    PlacementBatchProgress,
    PlacementEvidence,
}

impl RerunArtifact {
    pub(crate) fn file_name(self) -> &'static str {
        match self {
            Self::KeywordBatchProgress => KEYWORD_BATCH_PROGRESS_FILE,
            Self::TaxonomyBatchProgress => TAXONOMY_BATCH_PROGRESS_FILE,
            Self::PlacementBatchProgress => PLACEMENT_BATCH_PROGRESS_FILE,
            Self::PlacementEvidence => PLACEMENT_EVIDENCE_FILE,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::KeywordBatchProgress => "keyword batch progress",
            Self::TaxonomyBatchProgress => "taxonomy batch progress",
            Self::PlacementBatchProgress => "placement batch progress",
            Self::PlacementEvidence => "placement evidence",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerunImpact {
    pub(crate) start_stage: RunStage,
    pub(crate) previous_last_completed_stage: Option<RunStage>,
    pub(crate) cleared_stage_files: Vec<RunStage>,
    pub(crate) cleared_artifacts: Vec<RerunArtifact>,
    pub(crate) report_reset_sections: Vec<&'static str>,
}

impl RerunImpact {
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!(
                "Restart stage: {} | {}",
                rerun_stage_name(self.start_stage),
                self.start_stage.description()
            ),
            format!(
                "Saved progress before restart: {}",
                self.previous_last_completed_stage.map_or_else(
                    || "none (the run will restart from scratch)".to_string(),
                    |stage| format!("{} | {}", rerun_stage_name(stage), stage.description())
                )
            ),
            String::new(),
            "Stage files removed:".to_string(),
        ];

        if self.cleared_stage_files.is_empty() {
            lines.push("  none".to_string());
        } else {
            for stage in &self.cleared_stage_files {
                lines.push(format!(
                    "  {} | {}",
                    rerun_stage_name(*stage),
                    stage.description()
                ));
            }
        }

        lines.push(String::new());
        lines.push("Extra artifacts cleared:".to_string());
        if self.cleared_artifacts.is_empty() {
            lines.push("  none".to_string());
        } else {
            for artifact in &self.cleared_artifacts {
                lines.push(format!("  {}", artifact.label()));
            }
        }

        lines.push(String::new());
        lines.push("Report sections reset:".to_string());
        if self.report_reset_sections.is_empty() {
            lines.push("  none".to_string());
        } else {
            for section in &self.report_reset_sections {
                lines.push(format!("  {section}"));
            }
        }

        lines
    }
}

pub fn describe_rerun_impact(config: &AppConfig, start_stage: RunStage) -> Result<RerunImpact> {
    let stages = available_rerun_stages(config);
    let Some(start_index) = stages.iter().position(|stage| *stage == start_stage) else {
        return Err(AppError::Execution(format!(
            "stage '{}' is not available for this run",
            rerun_stage_name(start_stage)
        )));
    };

    let mut cleared_artifacts = Vec::new();
    if start_index
        <= stages
            .iter()
            .position(|stage| *stage == RunStage::ExtractKeywords)
            .unwrap_or(usize::MAX)
    {
        cleared_artifacts.push(RerunArtifact::KeywordBatchProgress);
    }
    if start_index
        <= stages
            .iter()
            .position(|stage| *stage == RunStage::SynthesizeCategories)
            .unwrap_or(usize::MAX)
    {
        cleared_artifacts.push(RerunArtifact::TaxonomyBatchProgress);
    }
    if start_index
        <= stages
            .iter()
            .position(|stage| *stage == RunStage::GeneratePlacements)
            .unwrap_or(usize::MAX)
    {
        cleared_artifacts.push(RerunArtifact::PlacementBatchProgress);
        cleared_artifacts.push(RerunArtifact::PlacementEvidence);
    }

    Ok(RerunImpact {
        start_stage,
        previous_last_completed_stage: start_index.checked_sub(1).map(|index| stages[index]),
        cleared_stage_files: stages
            .iter()
            .copied()
            .skip(start_index)
            .filter(|stage| stage.file_name().is_some())
            .collect(),
        cleared_artifacts,
        report_reset_sections: report_reset_sections(start_stage),
    })
}

pub(crate) fn rerun_stage_name(stage: RunStage) -> &'static str {
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

pub(crate) fn prepare_workspace_for_rerun(
    workspace: &mut crate::session::RunWorkspace,
    config: &AppConfig,
    start_stage: RunStage,
) -> Result<()> {
    let impact = describe_rerun_impact(config, start_stage)?;

    for stage in impact.cleared_stage_files.iter().copied() {
        workspace.remove_stage_file(stage)?;
    }
    workspace.set_last_completed_stage(impact.previous_last_completed_stage)?;

    for artifact in impact.cleared_artifacts {
        workspace.remove_artifact(artifact.file_name())?;
    }

    let mut report = workspace
        .load_report()?
        .unwrap_or_else(|| RunReport::new(config.dry_run));
    reset_report_for_rerun(&mut report, start_stage);
    workspace.save_report(&report)?;
    Ok(())
}

pub(crate) fn available_rerun_stages(config: &AppConfig) -> Vec<RunStage> {
    stage_sequence(config.rebuild && config.output.exists(), true)
}

pub(crate) fn validate_rerun_stage(stage: RunStage, stages: &[RunStage]) -> Result<RunStage> {
    stages
        .iter()
        .copied()
        .find(|candidate| *candidate == stage)
        .ok_or_else(|| {
            AppError::Execution(format!(
                "stage '{}' is not available for this run",
                rerun_stage_name(stage)
            ))
        })
}

fn reset_report_for_rerun(report: &mut RunReport, start_stage: RunStage) {
    match start_stage {
        RunStage::DiscoverInput
        | RunStage::DiscoverOutput
        | RunStage::Dedupe
        | RunStage::FilterSize
        | RunStage::ExtractText => {
            report.scanned = 0;
            report.processed = 0;
            report.skipped = 0;
            report.failed = 0;
            report.actions.clear();
            report.llm_usage.keywords = LlmUsageSummary::default();
            report.llm_usage.taxonomy = LlmUsageSummary::default();
            report.llm_usage.placements = LlmUsageSummary::default();
        }
        RunStage::BuildLlmClient | RunStage::ExtractKeywords => {
            report.actions.clear();
            report.llm_usage.keywords = LlmUsageSummary::default();
            report.llm_usage.taxonomy = LlmUsageSummary::default();
            report.llm_usage.placements = LlmUsageSummary::default();
        }
        RunStage::SynthesizeCategories => {
            report.actions.clear();
            report.llm_usage.taxonomy = LlmUsageSummary::default();
            report.llm_usage.placements = LlmUsageSummary::default();
        }
        RunStage::InspectOutput | RunStage::GeneratePlacements => {
            report.actions.clear();
            report.llm_usage.placements = LlmUsageSummary::default();
        }
        RunStage::BuildPlan => {
            report.actions.clear();
        }
        RunStage::ExecutePlan | RunStage::Completed => {}
    }
}

fn report_reset_sections(start_stage: RunStage) -> Vec<&'static str> {
    match start_stage {
        RunStage::DiscoverInput
        | RunStage::DiscoverOutput
        | RunStage::Dedupe
        | RunStage::FilterSize
        | RunStage::ExtractText => vec![
            "scan and extraction counters",
            "planned actions",
            "keyword LLM usage",
            "taxonomy LLM usage",
            "placement LLM usage",
        ],
        RunStage::BuildLlmClient | RunStage::ExtractKeywords => vec![
            "planned actions",
            "keyword LLM usage",
            "taxonomy LLM usage",
            "placement LLM usage",
        ],
        RunStage::SynthesizeCategories => vec![
            "planned actions",
            "taxonomy LLM usage",
            "placement LLM usage",
        ],
        RunStage::InspectOutput | RunStage::GeneratePlacements => {
            vec!["planned actions", "placement LLM usage"]
        }
        RunStage::BuildPlan => vec!["planned actions"],
        RunStage::ExecutePlan | RunStage::Completed => Vec::new(),
    }
}
