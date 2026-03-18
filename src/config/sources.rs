use std::{env, path::PathBuf};

use crate::{
    error::{AppError, Result},
    models::{LlmProvider, PlacementMode, TaxonomyMode},
};

use super::EnvConfig;

pub(super) fn env_config_from_process() -> Result<EnvConfig> {
    Ok(EnvConfig {
        input: env::var("SYP_INPUT").ok().map(PathBuf::from),
        output: env::var("SYP_OUTPUT").ok().map(PathBuf::from),
        recursive: parse_env_bool("SYP_RECURSIVE")?,
        max_file_size_mb: parse_env_u64("SYP_MAX_FILE_SIZE_MB")?,
        page_cutoff: parse_env_u8("SYP_PAGE_CUTOFF")?,
        pdf_extract_workers: parse_env_usize("SYP_PDF_EXTRACT_WORKERS")?,
        category_depth: parse_env_u8("SYP_CATEGORY_DEPTH")?,
        taxonomy_mode: parse_env_taxonomy_mode("SYP_TAXONOMY_MODE")?,
        taxonomy_batch_size: parse_env_usize("SYP_TAXONOMY_BATCH_SIZE")?,
        placement_batch_size: parse_env_usize("SYP_PLACEMENT_BATCH_SIZE")?,
        placement_mode: parse_env_placement_mode("SYP_PLACEMENT_MODE")?,
        rebuild: parse_env_bool("SYP_REBUILD")?,
        llm_provider: parse_env_provider("SYP_LLM_PROVIDER")?,
        llm_model: env::var("SYP_LLM_MODEL").ok(),
        llm_base_url: env::var("SYP_LLM_BASE_URL").ok(),
        api_key: env::var("SYP_API_KEY").ok(),
        keyword_batch_size: parse_env_usize("SYP_KEYWORD_BATCH_SIZE")?,
        batch_start_delay_ms: parse_env_u64("SYP_BATCH_START_DELAY_MS")?,
        subcategories_suggestion_number: parse_env_usize("SYP_SUBCATEGORIES_SUGGESTION_NUMBER")?,
    })
}

fn parse_env_bool(key: &str) -> Result<Option<bool>> {
    match env::var(key) {
        Ok(value) => parse_bool(key, &value).map(Some),
        Err(_) => Ok(None),
    }
}

fn parse_env_u64(key: &str) -> Result<Option<u64>> {
    match env::var(key) {
        Ok(value) => value
            .parse::<u64>()
            .map(Some)
            .map_err(|_| AppError::Config(format!("{key} must be a positive integer"))),
        Err(_) => Ok(None),
    }
}

fn parse_env_u8(key: &str) -> Result<Option<u8>> {
    match env::var(key) {
        Ok(value) => value
            .parse::<u8>()
            .map(Some)
            .map_err(|_| AppError::Config(format!("{key} must be an integer 0-255"))),
        Err(_) => Ok(None),
    }
}

fn parse_env_usize(key: &str) -> Result<Option<usize>> {
    match env::var(key) {
        Ok(value) => value
            .parse::<usize>()
            .map(Some)
            .map_err(|_| AppError::Config(format!("{key} must be a positive integer"))),
        Err(_) => Ok(None),
    }
}

fn parse_env_provider(key: &str) -> Result<Option<LlmProvider>> {
    match env::var(key) {
        Ok(value) => match value.to_ascii_lowercase().as_str() {
            "openai" => Ok(Some(LlmProvider::Openai)),
            "ollama" => Ok(Some(LlmProvider::Ollama)),
            "gemini" => Ok(Some(LlmProvider::Gemini)),
            _ => Err(AppError::Config(format!(
                "{key} must be one of: openai, ollama, gemini"
            ))),
        },
        Err(_) => Ok(None),
    }
}

fn parse_env_taxonomy_mode(key: &str) -> Result<Option<TaxonomyMode>> {
    match env::var(key) {
        Ok(value) => match value.to_ascii_lowercase().as_str() {
            "global" => Ok(Some(TaxonomyMode::Global)),
            "batch-merge" => Ok(Some(TaxonomyMode::BatchMerge)),
            _ => Err(AppError::Config(format!(
                "{key} must be one of: global, batch-merge"
            ))),
        },
        Err(_) => Ok(None),
    }
}

fn parse_env_placement_mode(key: &str) -> Result<Option<PlacementMode>> {
    match env::var(key) {
        Ok(value) => match value.to_ascii_lowercase().as_str() {
            "existing-only" => Ok(Some(PlacementMode::ExistingOnly)),
            "allow-new" => Ok(Some(PlacementMode::AllowNew)),
            _ => Err(AppError::Config(format!(
                "{key} must be one of: existing-only, allow-new"
            ))),
        },
        Err(_) => Ok(None),
    }
}

fn parse_bool(key: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(AppError::Config(format!(
            "{key} must be a boolean-like value"
        ))),
    }
}
