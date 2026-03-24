use std::path::PathBuf;

use crate::{
    llm::LlmProvider,
    papers::extract::ExtractorMode,
    papers::placement::PlacementMode,
    papers::taxonomy::{TaxonomyAssistance, TaxonomyMode},
};

#[derive(Debug, Clone, Default)]
pub struct RunOverrides {
    pub input: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub recursive: Option<bool>,
    pub max_file_size_mb: Option<u64>,
    pub page_cutoff: Option<u8>,
    pub pdf_extract_workers: Option<usize>,
    pub category_depth: Option<u8>,
    pub taxonomy_mode: Option<TaxonomyMode>,
    pub taxonomy_assistance: Option<TaxonomyAssistance>,
    pub taxonomy_batch_size: Option<usize>,
    pub reference_manifest_path: Option<PathBuf>,
    pub reference_top_k: Option<usize>,
    pub use_current_folder_tree: Option<bool>,
    pub placement_batch_size: Option<usize>,
    pub placement_mode: Option<PlacementMode>,
    pub rebuild: Option<bool>,
    pub apply: bool,
    pub llm_provider: Option<LlmProvider>,
    pub llm_model: Option<String>,
    pub llm_base_url: Option<String>,
    pub api_key: Option<String>,
    pub api_key_command: Option<String>,
    pub api_key_env: Option<String>,
    pub embedding_provider: Option<LlmProvider>,
    pub embedding_model: Option<String>,
    pub embedding_base_url: Option<String>,
    pub embedding_api_key: Option<String>,
    pub embedding_api_key_command: Option<String>,
    pub embedding_api_key_env: Option<String>,
    pub keyword_batch_size: Option<usize>,
    pub subcategories_suggestion_number: Option<usize>,
    pub verbosity: u8,
    pub quiet: bool,
}

#[derive(Debug, Clone)]
pub struct ExtractTextRequest {
    pub files: Vec<PathBuf>,
    pub page_cutoff: u8,
    pub extractor: ExtractorMode,
    pub pdf_extract_workers: usize,
    pub verbosity: u8,
}
