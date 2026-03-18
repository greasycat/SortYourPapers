use std::{
    fs,
    path::{Path, PathBuf},
};

use directories::BaseDirs;

use crate::{
    config::{
        DEFAULT_BATCH_START_DELAY_MS, DEFAULT_CATEGORY_DEPTH, DEFAULT_INPUT,
        DEFAULT_KEYWORD_BATCH_SIZE, DEFAULT_LLM_MODEL, DEFAULT_MAX_FILE_SIZE_MB, DEFAULT_OUTPUT,
        DEFAULT_PAGE_CUTOFF, DEFAULT_PDF_EXTRACT_WORKERS, DEFAULT_PLACEMENT_BATCH_SIZE,
        DEFAULT_REBUILD, DEFAULT_RECURSIVE, DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER,
        DEFAULT_TAXONOMY_BATCH_SIZE, FileConfig,
    },
    error::{AppError, Result},
};

pub(super) fn xdg_config_path() -> Option<PathBuf> {
    BaseDirs::new().map(|base| base.config_dir().join("sortyourpapers").join("config.toml"))
}

pub(super) fn xdg_cache_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|base| base.cache_dir().join("sortyourpapers"))
}

pub(super) fn init_xdg_config(force: bool) -> Result<PathBuf> {
    let Some(path) = xdg_config_path() else {
        return Err(AppError::Config(
            "could not resolve XDG config directory".to_string(),
        ));
    };

    write_default_config_at(&path, force)?;
    Ok(path)
}

pub(super) fn default_config_toml() -> String {
    format!(
        concat!(
            "# SortYourPapers default configuration\n",
            "# Priority: CLI > ENV > XDG > defaults\n",
            "\n",
            "input = \"{input}\"\n",
            "output = \"{output}\"\n",
            "recursive = {recursive}\n",
            "max_file_size_mb = {max_file_size_mb}\n",
            "page_cutoff = {page_cutoff}\n",
            "pdf_extract_workers = {pdf_extract_workers}\n",
            "category_depth = {category_depth}\n",
            "taxonomy_mode = \"batch-merge\"\n",
            "taxonomy_batch_size = {taxonomy_batch_size}\n",
            "placement_batch_size = {placement_batch_size}\n",
            "placement_mode = \"existing-only\"\n",
            "rebuild = {rebuild}\n",
            "\n",
            "# Default LLM settings\n",
            "llm_provider = \"gemini\"\n",
            "llm_model = \"{llm_model}\"\n",
            "keyword_batch_size = {keyword_batch_size}\n",
            "batch_start_delay_ms = {batch_start_delay_ms}\n",
            "subcategories_suggestion_number = {subcategories_suggestion_number}\n",
            "# llm_base_url = \"https://generativelanguage.googleapis.com/v1beta\"\n",
            "# api_key = \"\"\n"
        ),
        input = DEFAULT_INPUT,
        output = DEFAULT_OUTPUT,
        recursive = DEFAULT_RECURSIVE,
        max_file_size_mb = DEFAULT_MAX_FILE_SIZE_MB,
        page_cutoff = DEFAULT_PAGE_CUTOFF,
        pdf_extract_workers = DEFAULT_PDF_EXTRACT_WORKERS,
        category_depth = DEFAULT_CATEGORY_DEPTH,
        taxonomy_batch_size = DEFAULT_TAXONOMY_BATCH_SIZE,
        placement_batch_size = DEFAULT_PLACEMENT_BATCH_SIZE,
        rebuild = DEFAULT_REBUILD,
        llm_model = DEFAULT_LLM_MODEL,
        keyword_batch_size = DEFAULT_KEYWORD_BATCH_SIZE,
        batch_start_delay_ms = DEFAULT_BATCH_START_DELAY_MS,
        subcategories_suggestion_number = DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER,
    )
}

pub(super) fn load_xdg_config() -> Result<FileConfig> {
    let Some(path) = xdg_config_path() else {
        return Ok(FileConfig::default());
    };

    if !path.exists() {
        return Ok(FileConfig::default());
    }

    load_config_from_path(&path)
}

fn load_config_from_path(path: &Path) -> Result<FileConfig> {
    let raw = fs::read_to_string(path)?;
    let cfg: FileConfig = toml::from_str(&raw)?;
    Ok(cfg)
}

pub(super) fn write_default_config_at(path: &Path, force: bool) -> Result<()> {
    if path.exists() && !force {
        return Err(AppError::Config(format!(
            "config already exists at {} (use `init --force` to overwrite)",
            path.display()
        )));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, default_config_toml())?;
    Ok(())
}
