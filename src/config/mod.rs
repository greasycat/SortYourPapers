mod resolve;
mod sources;
mod xdg;

#[cfg(test)]
mod tests;

use serde::Deserialize;

use crate::{
    cli::{
        DEFAULT_BATCH_START_DELAY_MS, DEFAULT_CATEGORY_DEPTH, DEFAULT_INPUT,
        DEFAULT_KEYWORD_BATCH_SIZE, DEFAULT_LLM_MODEL, DEFAULT_LLM_PROVIDER,
        DEFAULT_MAX_FILE_SIZE_MB, DEFAULT_OUTPUT, DEFAULT_PAGE_CUTOFF,
        DEFAULT_PDF_EXTRACT_WORKERS, DEFAULT_PLACEMENT_BATCH_SIZE, DEFAULT_REBUILD,
        DEFAULT_RECURSIVE, DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER,
        DEFAULT_TAXONOMY_BATCH_SIZE,
    },
    domain::{AppConfig, LlmProvider, PlacementMode, TaxonomyMode},
    error::Result,
};
pub use crate::cli::{Cli, CliArgs, Commands, ExtractTextArgs, InitArgs, SessionCommands};

use std::path::PathBuf;

#[derive(Debug, Default, Deserialize, Clone)]
struct FileConfig {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    recursive: Option<bool>,
    max_file_size_mb: Option<u64>,
    page_cutoff: Option<u8>,
    pdf_extract_workers: Option<usize>,
    category_depth: Option<u8>,
    taxonomy_mode: Option<TaxonomyMode>,
    taxonomy_batch_size: Option<usize>,
    placement_batch_size: Option<usize>,
    placement_mode: Option<PlacementMode>,
    rebuild: Option<bool>,
    llm_provider: Option<LlmProvider>,
    llm_model: Option<String>,
    llm_base_url: Option<String>,
    api_key: Option<String>,
    keyword_batch_size: Option<usize>,
    batch_start_delay_ms: Option<u64>,
    subcategories_suggestion_number: Option<usize>,
}

#[derive(Debug, Default)]
struct EnvConfig {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    recursive: Option<bool>,
    max_file_size_mb: Option<u64>,
    page_cutoff: Option<u8>,
    pdf_extract_workers: Option<usize>,
    category_depth: Option<u8>,
    taxonomy_mode: Option<TaxonomyMode>,
    taxonomy_batch_size: Option<usize>,
    placement_batch_size: Option<usize>,
    placement_mode: Option<PlacementMode>,
    rebuild: Option<bool>,
    llm_provider: Option<LlmProvider>,
    llm_model: Option<String>,
    llm_base_url: Option<String>,
    api_key: Option<String>,
    keyword_batch_size: Option<usize>,
    batch_start_delay_ms: Option<u64>,
    subcategories_suggestion_number: Option<usize>,
}

/// Resolves the runtime configuration from CLI, environment, XDG config, and defaults.
///
/// # Errors
/// Returns an error when config sources cannot be loaded or the resolved
/// configuration contains invalid values.
pub fn resolve_config(cli: CliArgs) -> Result<AppConfig> {
    let file_cfg = xdg::load_xdg_config()?;
    let env_cfg = sources::env_config_from_process()?;
    resolve::resolve_from_sources(cli, env_cfg, file_cfg)
}

#[must_use]
pub fn xdg_config_path() -> Option<PathBuf> {
    xdg::xdg_config_path()
}

#[must_use]
pub fn xdg_cache_dir() -> Option<PathBuf> {
    xdg::xdg_cache_dir()
}

/// Initializes the default XDG configuration file.
///
/// # Errors
/// Returns an error when the XDG config directory cannot be resolved or the
/// file cannot be written.
pub fn init_xdg_config(force: bool) -> Result<PathBuf> {
    xdg::init_xdg_config(force)
}

#[must_use]
pub fn default_config_toml() -> String {
    xdg::default_config_toml()
}
