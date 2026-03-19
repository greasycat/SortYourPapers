use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};

use crate::{
    llm::LlmProvider, papers::extract::ExtractorMode, papers::placement::PlacementMode,
    papers::taxonomy::TaxonomyMode, session::RunStage,
};

pub(crate) const DEFAULT_INPUT: &str = ".";
pub(crate) const DEFAULT_OUTPUT: &str = "./sorted";
pub(crate) const DEFAULT_MAX_FILE_SIZE_MB: u64 = 16;
pub(crate) const DEFAULT_PAGE_CUTOFF: u8 = 1;
pub(crate) const DEFAULT_PDF_EXTRACT_WORKERS: usize = 8;
pub(crate) const DEFAULT_CATEGORY_DEPTH: u8 = 2;
pub(crate) const DEFAULT_KEYWORD_BATCH_SIZE: usize = 20;
pub(crate) const DEFAULT_BATCH_START_DELAY_MS: u64 = 100;
pub(crate) const DEFAULT_TAXONOMY_BATCH_SIZE: usize = 4;
pub(crate) const DEFAULT_PLACEMENT_BATCH_SIZE: usize = 10;
pub(crate) const DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER: usize = 5;
pub(crate) const DEFAULT_RECURSIVE: bool = false;
pub(crate) const DEFAULT_REBUILD: bool = false;
pub(crate) const DEFAULT_LLM_PROVIDER: LlmProvider = LlmProvider::Gemini;
pub(crate) const DEFAULT_LLM_MODEL: &str = "gemini-3-flash-preview";

#[derive(Debug, Parser)]
#[command(name = "sortyourpapers", version, about = "Sort PDFs with LLMs")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[command(flatten)]
    pub run: CliArgs,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Init(InitArgs),
    ExtractText(ExtractTextArgs),
    Session(SessionArgs),
}

#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct ExtractTextArgs {
    #[arg(value_name = "PDF", required = true)]
    pub files: Vec<PathBuf>,

    #[arg(short = 'p', long, default_value_t = DEFAULT_PAGE_CUTOFF)]
    pub page_cutoff: u8,

    #[arg(short = 'e', long, value_enum, default_value_t = ExtractorMode::Auto)]
    pub extractor: ExtractorMode,

    #[arg(long, default_value_t = DEFAULT_PDF_EXTRACT_WORKERS)]
    pub pdf_extract_workers: usize,

    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbosity: u8,
}

#[derive(Debug, Args)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub command: SessionCommands,
}

#[derive(Debug, Subcommand)]
pub enum SessionCommands {
    Resume(ResumeArgs),
    Rerun(RerunArgs),
    Review(SessionReviewArgs),
    List,
    Remove(SessionRemoveArgs),
    Clear,
}

#[derive(Debug, Args)]
pub struct ResumeArgs {
    #[arg(value_name = "RUN_ID")]
    pub run_id: Option<String>,

    #[arg(short = 'a', long, action = ArgAction::SetTrue)]
    pub apply: bool,

    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbosity: u8,

    #[arg(short = 'q', long, action = ArgAction::SetTrue)]
    pub quiet: bool,
}

#[derive(Debug, Args)]
pub struct RerunArgs {
    #[arg(value_name = "RUN_ID")]
    pub run_id: Option<String>,

    #[arg(short = 's', long, value_enum)]
    pub stage: Option<RunStage>,

    #[arg(short = 'a', long, action = ArgAction::SetTrue)]
    pub apply: bool,

    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbosity: u8,

    #[arg(short = 'q', long, action = ArgAction::SetTrue)]
    pub quiet: bool,
}

#[derive(Debug, Args)]
pub struct SessionReviewArgs {
    #[arg(value_name = "RUN_ID")]
    pub run_id: Option<String>,
}

#[derive(Debug, Args)]
pub struct SessionRemoveArgs {
    #[arg(value_name = "RUN_ID")]
    pub run_ids: Vec<String>,
}

#[derive(Debug, Parser, Clone)]
pub struct CliArgs {
    #[arg(short = 'i', long)]
    pub input: Option<PathBuf>,

    #[arg(short = 'o', long)]
    pub output: Option<PathBuf>,

    #[arg(short = 'r', long, num_args = 0..=1, default_missing_value = "true")]
    pub recursive: Option<bool>,

    #[arg(short = 's', long)]
    pub max_file_size_mb: Option<u64>,

    #[arg(short = 'p', long)]
    pub page_cutoff: Option<u8>,

    #[arg(long)]
    pub pdf_extract_workers: Option<usize>,

    #[arg(short = 'd', long)]
    pub category_depth: Option<u8>,

    #[arg(long)]
    pub taxonomy_mode: Option<TaxonomyMode>,

    #[arg(long)]
    pub taxonomy_batch_size: Option<usize>,

    #[arg(long)]
    pub placement_batch_size: Option<usize>,

    #[arg(short = 'M', long)]
    pub placement_mode: Option<PlacementMode>,

    #[arg(short = 'R', long, num_args = 0..=1, default_missing_value = "true")]
    pub rebuild: Option<bool>,

    #[arg(short = 'a', long, action = ArgAction::SetTrue)]
    pub apply: bool,

    #[arg(short = 'P', long)]
    pub llm_provider: Option<LlmProvider>,

    #[arg(short = 'm', long)]
    pub llm_model: Option<String>,

    #[arg(short = 'u', long)]
    pub llm_base_url: Option<String>,

    #[arg(short = 'k', long)]
    pub api_key: Option<String>,

    #[arg(long)]
    pub keyword_batch_size: Option<usize>,

    #[arg(long)]
    pub subcategories_suggestion_number: Option<usize>,

    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbosity: u8,

    #[arg(short = 'q', long, action = ArgAction::SetTrue)]
    pub quiet: bool,
}
