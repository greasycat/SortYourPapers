mod resolve;
mod sources;
mod xdg;

#[cfg(test)]
mod tests;

use std::{env, path::PathBuf, process::Command};

use crate::{
    error::Result, inputs::RunOverrides, llm::LlmProvider, papers::placement::PlacementMode,
    papers::taxonomy::TaxonomyMode,
};
use serde::{Deserialize, Serialize};

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
    pub use_current_folder_tree: bool,
    pub placement_batch_size: usize,
    pub placement_mode: PlacementMode,
    pub rebuild: bool,
    pub dry_run: bool,
    pub llm_provider: LlmProvider,
    pub llm_model: String,
    pub llm_base_url: Option<String>,
    pub api_key: Option<ApiKeySource>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", content = "value", rename_all = "kebab-case")]
pub enum ApiKeySource {
    Text(String),
    Command(String),
    Env(String),
}

impl ApiKeySource {
    pub fn resolve(&self) -> Result<String> {
        match self {
            Self::Text(value) => resolve_api_key_text(value),
            Self::Command(command) => resolve_api_key_command(command),
            Self::Env(name) => resolve_api_key_env(name),
        }
    }
}

impl AppConfig {
    pub fn resolved_api_key(&self) -> Result<Option<String>> {
        self.api_key.as_ref().map(ApiKeySource::resolve).transpose()
    }
}

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
    use_current_folder_tree: Option<bool>,
    placement_batch_size: Option<usize>,
    placement_mode: Option<PlacementMode>,
    rebuild: Option<bool>,
    llm_provider: Option<LlmProvider>,
    llm_model: Option<String>,
    llm_base_url: Option<String>,
    api_key: Option<ApiKeySource>,
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
    use_current_folder_tree: Option<bool>,
    placement_batch_size: Option<usize>,
    placement_mode: Option<PlacementMode>,
    rebuild: Option<bool>,
    llm_provider: Option<LlmProvider>,
    llm_model: Option<String>,
    llm_base_url: Option<String>,
    api_key: Option<ApiKeySource>,
    keyword_batch_size: Option<usize>,
    batch_start_delay_ms: Option<u64>,
    subcategories_suggestion_number: Option<usize>,
}

/// Resolves the runtime configuration from explicit overrides, environment,
/// XDG config, and defaults.
///
/// # Errors
/// Returns an error when config sources cannot be loaded or the resolved
/// configuration contains invalid values.
pub fn resolve_config(overrides: RunOverrides) -> Result<AppConfig> {
    let file_cfg = xdg::load_xdg_config()?;
    let env_cfg = sources::env_config_from_process()?;
    resolve::resolve_from_sources(overrides, env_cfg, file_cfg)
}

#[must_use]
pub fn xdg_config_path() -> Option<PathBuf> {
    xdg::xdg_config_path()
}

#[must_use]
pub fn xdg_cache_dir() -> Option<PathBuf> {
    xdg::xdg_cache_dir()
}

#[must_use]
pub fn xdg_testset_cache_dir() -> Option<PathBuf> {
    xdg::xdg_testset_cache_dir()
}

#[must_use]
pub fn xdg_data_dir() -> Option<PathBuf> {
    xdg::xdg_data_dir()
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

/// Saves the current runtime configuration as the XDG config file.
///
/// # Errors
/// Returns an error when the XDG config path cannot be resolved or the file
/// cannot be written.
pub fn save_xdg_config(config: &AppConfig) -> Result<PathBuf> {
    xdg::save_xdg_config(config)
}

fn resolve_api_key_text(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(crate::error::AppError::Config(
            "api key text value is empty".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn resolve_api_key_env(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(crate::error::AppError::Config(
            "api key env variable name is empty".to_string(),
        ));
    }

    let value = env::var(trimmed).map_err(|_| {
        crate::error::AppError::Config(format!("api key env variable {trimmed} is not set"))
    })?;
    resolve_api_key_text(&value)
}

fn resolve_api_key_command(command: &str) -> Result<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err(crate::error::AppError::Config(
            "api key command is empty".to_string(),
        ));
    }

    #[cfg(target_family = "windows")]
    let output = Command::new("cmd").args(["/C", trimmed]).output()?;
    #[cfg(not(target_family = "windows"))]
    let output = Command::new("sh").args(["-lc", trimmed]).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("command exited with status {}", output.status)
        } else {
            stderr
        };
        return Err(crate::error::AppError::Config(format!(
            "api key command failed: {detail}"
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    resolve_api_key_text(stdout.trim())
}
