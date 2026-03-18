use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::taxonomy::{LlmProvider, PlacementMode, TaxonomyMode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub input: PathBuf,
    pub output: PathBuf,
    pub recursive: bool,
    pub max_file_size_mb: u64,
    pub page_cutoff: u8,
    pub pdf_extract_workers: usize,
    pub category_depth: u8,
    pub taxonomy_mode: TaxonomyMode,
    pub taxonomy_batch_size: usize,
    pub placement_batch_size: usize,
    pub placement_mode: PlacementMode,
    pub rebuild: bool,
    pub dry_run: bool,
    pub llm_provider: LlmProvider,
    pub llm_model: String,
    pub llm_base_url: Option<String>,
    pub api_key: Option<String>,
    pub keyword_batch_size: usize,
    pub batch_start_delay_ms: u64,
    pub subcategories_suggestion_number: usize,
    #[serde(default)]
    pub verbose: bool,
    #[serde(default)]
    pub debug: bool,
    #[serde(default)]
    pub quiet: bool,
}
