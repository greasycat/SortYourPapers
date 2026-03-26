use std::path::PathBuf;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{
    papers::taxonomy::CategoryTree,
    papers::{PaperText, PdfCandidate},
    report::RunReport,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum RunStage {
    DiscoverInput,
    DiscoverOutput,
    Dedupe,
    FilterSize,
    ExtractText,
    BuildLlmClient,
    ExtractKeywords,
    SynthesizeCategories,
    InspectOutput,
    GeneratePlacements,
    BuildPlan,
    ExecutePlan,
    #[value(skip)]
    Completed,
}

impl RunStage {
    pub fn description(self) -> &'static str {
        match self {
            Self::DiscoverInput => "Discover input PDFs",
            Self::DiscoverOutput => "Discover existing output PDFs",
            Self::Dedupe => "Deduplicate candidate PDFs",
            Self::FilterSize => "Filter oversized PDFs",
            Self::ExtractText => "Extract text and preprocess papers",
            Self::BuildLlmClient => "Build LLM client",
            Self::ExtractKeywords => "Extract keywords",
            Self::SynthesizeCategories => "Synthesize categories",
            Self::InspectOutput => "Inspect merged taxonomy",
            Self::GeneratePlacements => "Generate placements",
            Self::BuildPlan => "Build file move plan",
            Self::ExecutePlan => "Execute plan",
            Self::Completed => "Complete run",
        }
    }

    pub(crate) fn file_name(self) -> Option<&'static str> {
        match self {
            Self::DiscoverInput => Some("01-discover-input.json"),
            Self::DiscoverOutput => Some("02-discover-output.json"),
            Self::Dedupe => Some("03-dedupe.json"),
            Self::FilterSize => Some("04-filter-size.json"),
            Self::ExtractText => Some("05-extract-text.json"),
            Self::BuildLlmClient => None,
            Self::ExtractKeywords => Some("06-extract-keywords.json"),
            Self::SynthesizeCategories => Some("07-synthesize-categories.json"),
            Self::InspectOutput => Some("08-inspect-output.json"),
            Self::GeneratePlacements => Some("09-generate-placements.json"),
            Self::BuildPlan => Some("10-build-plan.json"),
            Self::ExecutePlan => None,
            Self::Completed => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageFailure {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterSizeState {
    pub accepted: Vec<PdfCandidate>,
    pub skipped: Vec<PdfCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractTextState {
    pub papers: Vec<PaperText>,
    pub failures: Vec<StageFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RunManifest {
    pub(crate) run_id: String,
    pub(crate) created_unix_ms: u128,
    pub(crate) cwd: PathBuf,
    pub(crate) last_completed_stage: Option<RunStage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub run_id: String,
    pub created_unix_ms: u128,
    pub cwd: PathBuf,
    pub last_completed_stage: Option<RunStage>,
    pub is_latest: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionConfigSummary {
    pub dry_run: bool,
    pub llm_provider: String,
    pub llm_model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStatusSummary {
    pub is_completed: bool,
    pub is_incomplete: bool,
    pub is_failed_looking: bool,
}

#[derive(Debug, Clone)]
pub struct SessionDetails {
    pub run: RunSummary,
    pub config: SessionConfigSummary,
    pub status: SessionStatusSummary,
    pub report: Option<RunReport>,
    pub taxonomy: Option<Vec<CategoryTree>>,
    pub available_stage_artifacts: Vec<RunStage>,
}

#[derive(Debug, Clone)]
pub struct RunWorkspace {
    pub(crate) root_dir: PathBuf,
    pub(crate) manifest: RunManifest,
}
