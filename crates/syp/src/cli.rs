use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};
use syp_core::{
    defaults::{DEFAULT_PAGE_CUTOFF, DEFAULT_PDF_EXTRACT_WORKERS},
    inputs::{ExtractTextRequest, RunOverrides},
    llm::LlmProvider,
    papers::extract::ExtractorMode,
    papers::placement::{PlacementAssistance, PlacementMode},
    papers::taxonomy::{TaxonomyAssistance, TaxonomyMode},
    session::RunStage,
};

#[derive(Debug, Parser)]
#[command(name = "syp", version, about = "Sort PDFs with LLMs")]
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
    Reference(ReferenceArgs),
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

#[derive(Debug, Args)]
pub struct ReferenceArgs {
    #[command(subcommand)]
    pub command: ReferenceCommands,
}

#[derive(Debug, Subcommand)]
pub enum ReferenceCommands {
    Index(ReferenceIndexArgs),
}

#[derive(Debug, Args)]
pub struct ReferenceIndexArgs {
    #[arg(long)]
    pub manifest: Option<PathBuf>,

    #[arg(long, action = ArgAction::SetTrue)]
    pub force: bool,
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
    pub taxonomy_assistance: Option<TaxonomyAssistance>,

    #[arg(long)]
    pub taxonomy_batch_size: Option<usize>,

    #[arg(long)]
    pub reference_manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub reference_top_k: Option<usize>,

    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub use_current_folder_tree: Option<bool>,

    #[arg(long)]
    pub placement_batch_size: Option<usize>,

    #[arg(long)]
    pub placement_assistance: Option<PlacementAssistance>,

    #[arg(short = 'M', long)]
    pub placement_mode: Option<PlacementMode>,

    #[arg(long)]
    pub placement_reference_top_k: Option<usize>,

    #[arg(long)]
    pub placement_candidate_top_k: Option<usize>,

    #[arg(long)]
    pub placement_min_similarity: Option<f32>,

    #[arg(long)]
    pub placement_min_margin: Option<f32>,

    #[arg(long)]
    pub placement_min_reference_support: Option<usize>,

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

    #[arg(short = 'k', long, conflicts_with_all = ["api_key_command", "api_key_env"])]
    pub api_key: Option<String>,

    #[arg(long, conflicts_with_all = ["api_key", "api_key_env"])]
    pub api_key_command: Option<String>,

    #[arg(long, conflicts_with_all = ["api_key", "api_key_command"])]
    pub api_key_env: Option<String>,

    #[arg(long)]
    pub embedding_provider: Option<LlmProvider>,

    #[arg(long)]
    pub embedding_model: Option<String>,

    #[arg(long)]
    pub embedding_base_url: Option<String>,

    #[arg(long, conflicts_with_all = ["embedding_api_key_command", "embedding_api_key_env"])]
    pub embedding_api_key: Option<String>,

    #[arg(long, conflicts_with_all = ["embedding_api_key", "embedding_api_key_env"])]
    pub embedding_api_key_command: Option<String>,

    #[arg(long, conflicts_with_all = ["embedding_api_key", "embedding_api_key_command"])]
    pub embedding_api_key_env: Option<String>,

    #[arg(long)]
    pub keyword_batch_size: Option<usize>,

    #[arg(long)]
    pub subcategories_suggestion_number: Option<usize>,

    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbosity: u8,

    #[arg(short = 'q', long, action = ArgAction::SetTrue)]
    pub quiet: bool,
}

impl CliArgs {
    pub fn into_run_overrides(self) -> RunOverrides {
        RunOverrides {
            input: self.input,
            output: self.output,
            recursive: self.recursive,
            max_file_size_mb: self.max_file_size_mb,
            page_cutoff: self.page_cutoff,
            pdf_extract_workers: self.pdf_extract_workers,
            category_depth: self.category_depth,
            taxonomy_mode: self.taxonomy_mode,
            taxonomy_assistance: self.taxonomy_assistance,
            taxonomy_batch_size: self.taxonomy_batch_size,
            reference_manifest_path: self.reference_manifest_path,
            reference_top_k: self.reference_top_k,
            use_current_folder_tree: self.use_current_folder_tree,
            placement_batch_size: self.placement_batch_size,
            placement_assistance: self.placement_assistance,
            placement_mode: self.placement_mode,
            placement_reference_top_k: self.placement_reference_top_k,
            placement_candidate_top_k: self.placement_candidate_top_k,
            placement_min_similarity: self.placement_min_similarity,
            placement_min_margin: self.placement_min_margin,
            placement_min_reference_support: self.placement_min_reference_support,
            rebuild: self.rebuild,
            apply: self.apply,
            llm_provider: self.llm_provider,
            llm_model: self.llm_model,
            llm_base_url: self.llm_base_url,
            api_key: self.api_key,
            api_key_command: self.api_key_command,
            api_key_env: self.api_key_env,
            embedding_provider: self.embedding_provider,
            embedding_model: self.embedding_model,
            embedding_base_url: self.embedding_base_url,
            embedding_api_key: self.embedding_api_key,
            embedding_api_key_command: self.embedding_api_key_command,
            embedding_api_key_env: self.embedding_api_key_env,
            keyword_batch_size: self.keyword_batch_size,
            subcategories_suggestion_number: self.subcategories_suggestion_number,
            verbosity: self.verbosity,
            quiet: self.quiet,
        }
    }
}

impl ExtractTextArgs {
    pub fn into_request(self) -> ExtractTextRequest {
        ExtractTextRequest {
            files: self.files,
            page_cutoff: self.page_cutoff,
            extractor: self.extractor,
            pdf_extract_workers: self.pdf_extract_workers,
            verbosity: self.verbosity,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Commands, ReferenceCommands, SessionCommands};
    use syp_core::{papers::extract::ExtractorMode, session::RunStage};

    #[test]
    fn parses_extract_text_subcommand() {
        let cli = Cli::parse_from([
            "syp",
            "extract-text",
            "--page-cutoff",
            "2",
            "--extractor",
            "pdf-oxide",
            "-vv",
            "/tmp/a.pdf",
            "/tmp/b.pdf",
        ]);

        match cli.command {
            Some(Commands::ExtractText(args)) => {
                assert_eq!(args.page_cutoff, 2);
                assert_eq!(args.extractor, ExtractorMode::PdfOxide);
                assert_eq!(args.pdf_extract_workers, 8);
                assert_eq!(args.verbosity, 2);
                assert_eq!(args.files.len(), 2);
            }
            _ => panic!("expected extract-text command"),
        }
    }

    #[test]
    fn parses_reference_index_subcommand() {
        let cli = Cli::parse_from([
            "syp",
            "reference",
            "index",
            "--manifest",
            "assets/testsets/scijudgebench-diverse.toml",
            "--force",
        ]);

        match cli.command {
            Some(Commands::Reference(args)) => match args.command {
                ReferenceCommands::Index(index) => {
                    assert_eq!(
                        index.manifest,
                        Some("assets/testsets/scijudgebench-diverse.toml".into())
                    );
                    assert!(index.force);
                }
            },
            other => panic!("expected reference index command, got {other:?}"),
        }
    }

    #[test]
    fn parses_session_rerun_subcommand_with_stage() {
        let cli = Cli::parse_from([
            "syp",
            "session",
            "rerun",
            "--stage",
            "extract-keywords",
            "--apply",
            "-v",
            "run-123",
        ]);

        match cli.command {
            Some(Commands::Session(args)) => match args.command {
                SessionCommands::Rerun(args) => {
                    assert_eq!(args.run_id.as_deref(), Some("run-123"));
                    assert_eq!(args.stage, Some(RunStage::ExtractKeywords));
                    assert!(args.apply);
                    assert_eq!(args.verbosity, 1);
                    assert!(!args.quiet);
                }
                _ => panic!("expected session rerun command"),
            },
            _ => panic!("expected session command"),
        }
    }

    #[test]
    fn rejects_bare_session_command() {
        let err = Cli::try_parse_from(["syp", "session"])
            .expect_err("session should require a subcommand");

        assert!(err.to_string().contains("subcommand"));
    }
}
