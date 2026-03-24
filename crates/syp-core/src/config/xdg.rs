use std::{
    env, fs,
    path::{Path, PathBuf},
};

use directories::BaseDirs;
use serde::Serialize;

use crate::{
    config::{ApiKeySource, AppConfig, FileConfig},
    defaults::{
        DEFAULT_BATCH_START_DELAY_MS, DEFAULT_CATEGORY_DEPTH, DEFAULT_INPUT,
        DEFAULT_KEYWORD_BATCH_SIZE, DEFAULT_LLM_MODEL, DEFAULT_MAX_FILE_SIZE_MB, DEFAULT_OUTPUT,
        DEFAULT_PAGE_CUTOFF, DEFAULT_PDF_EXTRACT_WORKERS, DEFAULT_PLACEMENT_BATCH_SIZE,
        DEFAULT_REBUILD, DEFAULT_RECURSIVE, DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER,
        DEFAULT_TAXONOMY_BATCH_SIZE, DEFAULT_USE_CURRENT_FOLDER_TREE,
    },
    error::{AppError, Result},
};

const DEV_CONFIG_FILE: &str = "dev.toml";
const TESTSETS_DIR: &str = "testsets";

#[derive(Default, serde::Deserialize)]
struct DevConfig {
    #[serde(default)]
    testsets: DevTestsetsConfig,
}

#[derive(Default, serde::Deserialize)]
struct DevTestsetsConfig {
    cache_dir: Option<PathBuf>,
}

pub(super) fn xdg_config_path() -> Option<PathBuf> {
    BaseDirs::new().map(|base| base.config_dir().join("sortyourpapers").join("config.toml"))
}

pub(super) fn xdg_cache_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|base| base.cache_dir().join("sortyourpapers"))
}

pub(super) fn xdg_data_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|base| base.data_dir().join("sortyourpapers"))
}

pub(super) fn shared_testset_cache_dir() -> Result<PathBuf> {
    if let Ok(current_dir) = env::current_dir() {
        if let Some(path) = shared_testset_cache_dir_from(&current_dir)? {
            return Ok(path);
        }
    }

    if let Some(path) = shared_testset_cache_dir_from(Path::new(env!("CARGO_MANIFEST_DIR")))? {
        return Ok(path);
    }

    default_testset_cache_dir()
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

pub(super) fn save_xdg_config(config: &AppConfig) -> Result<PathBuf> {
    let Some(path) = xdg_config_path() else {
        return Err(AppError::Config(
            "could not resolve XDG config directory".to_string(),
        ));
    };

    write_saved_config_at(&path, config)?;
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
            "use_current_folder_tree = {use_current_folder_tree}\n",
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
            "# api_key = {{ source = \"env\", value = \"OPENAI_API_KEY\" }}\n"
        ),
        input = DEFAULT_INPUT,
        output = DEFAULT_OUTPUT,
        recursive = DEFAULT_RECURSIVE,
        max_file_size_mb = DEFAULT_MAX_FILE_SIZE_MB,
        page_cutoff = DEFAULT_PAGE_CUTOFF,
        pdf_extract_workers = DEFAULT_PDF_EXTRACT_WORKERS,
        category_depth = DEFAULT_CATEGORY_DEPTH,
        taxonomy_batch_size = DEFAULT_TAXONOMY_BATCH_SIZE,
        use_current_folder_tree = DEFAULT_USE_CURRENT_FOLDER_TREE,
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

fn load_dev_config_from_path(path: &Path) -> Result<DevConfig> {
    let raw = fs::read_to_string(path)?;
    let cfg: DevConfig = toml::from_str(&raw)?;
    Ok(cfg)
}

pub(super) fn shared_testset_cache_dir_from(start: &Path) -> Result<Option<PathBuf>> {
    let Some(dev_config_path) = find_dev_config_path(start) else {
        return Ok(None);
    };

    let dev_config = load_dev_config_from_path(&dev_config_path)?;
    let relative = dev_config.testsets.cache_dir.ok_or_else(|| {
        AppError::Config(format!(
            "missing [testsets].cache_dir in {}",
            dev_config_path.display()
        ))
    })?;
    let root = dev_config_path.parent().ok_or_else(|| {
        AppError::Config(format!(
            "could not resolve parent directory for {}",
            dev_config_path.display()
        ))
    })?;
    Ok(Some(root.join(relative)))
}

pub(super) fn default_testset_cache_dir() -> Result<PathBuf> {
    let Some(cache_root) = xdg_cache_dir() else {
        return Err(AppError::Config(
            "could not resolve XDG cache directory".to_string(),
        ));
    };
    Ok(cache_root.join(TESTSETS_DIR))
}

fn find_dev_config_path(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let candidate = dir.join(DEV_CONFIG_FILE);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
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

pub(super) fn write_saved_config_at(path: &Path, config: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, saved_config_toml(config)?)?;
    Ok(())
}

fn saved_config_toml(config: &AppConfig) -> Result<String> {
    let serialized = toml::to_string(&PersistedConfig::from(config))
        .map_err(|err| AppError::Config(format!("failed to serialize config: {err}")))?;
    Ok(format!(
        "# SortYourPapers saved configuration\n# Generated from the TUI run form\n\n{serialized}"
    ))
}

#[derive(Serialize)]
struct PersistedConfig<'a> {
    input: &'a Path,
    output: &'a Path,
    recursive: bool,
    max_file_size_mb: u64,
    page_cutoff: u8,
    pdf_extract_workers: usize,
    category_depth: u8,
    taxonomy_mode: crate::papers::taxonomy::TaxonomyMode,
    taxonomy_batch_size: usize,
    use_current_folder_tree: bool,
    placement_batch_size: usize,
    placement_mode: crate::papers::placement::PlacementMode,
    rebuild: bool,
    llm_provider: crate::llm::LlmProvider,
    llm_model: &'a str,
    llm_base_url: Option<&'a str>,
    api_key: Option<&'a ApiKeySource>,
    keyword_batch_size: usize,
    batch_start_delay_ms: u64,
    subcategories_suggestion_number: usize,
}

impl<'a> From<&'a AppConfig> for PersistedConfig<'a> {
    fn from(config: &'a AppConfig) -> Self {
        Self {
            input: &config.input,
            output: &config.output,
            recursive: config.recursive,
            max_file_size_mb: config.max_file_size_mb,
            page_cutoff: config.page_cutoff,
            pdf_extract_workers: config.pdf_extract_workers,
            category_depth: config.category_depth,
            taxonomy_mode: config.taxonomy_mode,
            taxonomy_batch_size: config.taxonomy_batch_size,
            use_current_folder_tree: config.use_current_folder_tree,
            placement_batch_size: config.placement_batch_size,
            placement_mode: config.placement_mode,
            rebuild: config.rebuild,
            llm_provider: config.llm_provider,
            llm_model: config.llm_model.as_str(),
            llm_base_url: config.llm_base_url.as_deref(),
            api_key: config.api_key.as_ref(),
            keyword_batch_size: config.keyword_batch_size,
            batch_start_delay_ms: config.batch_start_delay_ms,
            subcategories_suggestion_number: config.subcategories_suggestion_number,
        }
    }
}
