use std::{
    env, fs,
    path::{Path, PathBuf},
};

use clap::{ArgAction, Args, Parser, Subcommand};
use directories::BaseDirs;
use serde::Deserialize;

use crate::{
    error::{AppError, Result},
    models::{AppConfig, LlmProvider, PlacementMode, TaxonomyMode},
    pdf_extract::ExtractorMode,
};

const DEFAULT_INPUT: &str = ".";
const DEFAULT_OUTPUT: &str = "./sorted";
const DEFAULT_MAX_FILE_SIZE_MB: u64 = 16;
const DEFAULT_PAGE_CUTOFF: u8 = 1;
const DEFAULT_PDF_EXTRACT_WORKERS: usize = 8;
const DEFAULT_CATEGORY_DEPTH: u8 = 2;

const DEFAULT_KEYWORD_BATCH_SIZE: usize = 20;
const DEFAULT_BATCH_START_DELAY_MS: u64 = 100;
const DEFAULT_TAXONOMY_BATCH_SIZE: usize = 4;
const DEFAULT_PLACEMENT_BATCH_SIZE: usize = 10;

const DEFAULT_RECURSIVE: bool = false;
const DEFAULT_REBUILD: bool = false;

const DEFAULT_LLM_PROVIDER: LlmProvider = LlmProvider::Gemini;
const DEFAULT_LLM_MODEL: &str = "gemini-3-flash-preview";

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
    Resume(ResumeArgs),
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

    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbosity: u8,

    #[arg(short = 'q', long, action = ArgAction::SetTrue)]
    pub quiet: bool,
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
    placement_batch_size: Option<usize>,
    placement_mode: Option<PlacementMode>,
    rebuild: Option<bool>,
    llm_provider: Option<LlmProvider>,
    llm_model: Option<String>,
    llm_base_url: Option<String>,
    api_key: Option<String>,
    keyword_batch_size: Option<usize>,
    batch_start_delay_ms: Option<u64>,
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
}

impl EnvConfig {
    fn from_env() -> Result<Self> {
        Ok(Self {
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
        })
    }
}

pub fn resolve_config(cli: CliArgs) -> Result<AppConfig> {
    let file_cfg = load_xdg_config()?;
    let env_cfg = EnvConfig::from_env()?;
    resolve_from_sources(cli, env_cfg, file_cfg)
}

fn resolve_from_sources(
    cli: CliArgs,
    env_cfg: EnvConfig,
    file_cfg: FileConfig,
) -> Result<AppConfig> {
    let verbosity = normalize_verbosity(cli.verbosity);
    let input = cli
        .input
        .or(env_cfg.input)
        .or(file_cfg.input)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_INPUT));

    let output = cli
        .output
        .or(env_cfg.output)
        .or(file_cfg.output)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_OUTPUT));

    let recursive = cli
        .recursive
        .or(env_cfg.recursive)
        .or(file_cfg.recursive)
        .unwrap_or(DEFAULT_RECURSIVE);

    let max_file_size_mb = cli
        .max_file_size_mb
        .or(env_cfg.max_file_size_mb)
        .or(file_cfg.max_file_size_mb)
        .unwrap_or(DEFAULT_MAX_FILE_SIZE_MB);

    let page_cutoff = cli
        .page_cutoff
        .or(env_cfg.page_cutoff)
        .or(file_cfg.page_cutoff)
        .unwrap_or(DEFAULT_PAGE_CUTOFF);

    let pdf_extract_workers = cli
        .pdf_extract_workers
        .or(env_cfg.pdf_extract_workers)
        .or(file_cfg.pdf_extract_workers)
        .unwrap_or(DEFAULT_PDF_EXTRACT_WORKERS);

    let category_depth = cli
        .category_depth
        .or(env_cfg.category_depth)
        .or(file_cfg.category_depth)
        .unwrap_or(DEFAULT_CATEGORY_DEPTH);

    let taxonomy_mode = cli
        .taxonomy_mode
        .or(env_cfg.taxonomy_mode)
        .or(file_cfg.taxonomy_mode)
        .unwrap_or_default();

    let taxonomy_batch_size = cli
        .taxonomy_batch_size
        .or(env_cfg.taxonomy_batch_size)
        .or(file_cfg.taxonomy_batch_size)
        .unwrap_or(DEFAULT_TAXONOMY_BATCH_SIZE);

    let placement_batch_size = cli
        .placement_batch_size
        .or(env_cfg.placement_batch_size)
        .or(file_cfg.placement_batch_size)
        .unwrap_or(DEFAULT_PLACEMENT_BATCH_SIZE);

    let placement_mode = cli
        .placement_mode
        .or(env_cfg.placement_mode)
        .or(file_cfg.placement_mode)
        .unwrap_or_default();

    let rebuild = cli
        .rebuild
        .or(env_cfg.rebuild)
        .or(file_cfg.rebuild)
        .unwrap_or(DEFAULT_REBUILD);

    let dry_run = !cli.apply;

    let llm_provider = cli
        .llm_provider
        .or(env_cfg.llm_provider)
        .or(file_cfg.llm_provider)
        .unwrap_or(DEFAULT_LLM_PROVIDER);

    let llm_model = cli
        .llm_model
        .or(env_cfg.llm_model)
        .or(file_cfg.llm_model)
        .unwrap_or_else(|| DEFAULT_LLM_MODEL.to_string());

    let llm_base_url = cli
        .llm_base_url
        .or(env_cfg.llm_base_url)
        .or(file_cfg.llm_base_url);
    let api_key = cli.api_key.or(env_cfg.api_key).or(file_cfg.api_key);
    let keyword_batch_size = cli
        .keyword_batch_size
        .or(env_cfg.keyword_batch_size)
        .or(file_cfg.keyword_batch_size)
        .unwrap_or(DEFAULT_KEYWORD_BATCH_SIZE);
    let batch_start_delay_ms = env_cfg
        .batch_start_delay_ms
        .or(file_cfg.batch_start_delay_ms)
        .unwrap_or(DEFAULT_BATCH_START_DELAY_MS);

    if max_file_size_mb == 0 {
        return Err(AppError::Validation(
            "max_file_size_mb must be greater than 0".to_string(),
        ));
    }
    if page_cutoff == 0 {
        return Err(AppError::Validation(
            "page_cutoff must be greater than 0".to_string(),
        ));
    }
    if pdf_extract_workers == 0 {
        return Err(AppError::Validation(
            "pdf_extract_workers must be greater than 0".to_string(),
        ));
    }
    if category_depth == 0 {
        return Err(AppError::Validation(
            "category_depth must be greater than 0".to_string(),
        ));
    }
    if taxonomy_batch_size == 0 {
        return Err(AppError::Validation(
            "taxonomy_batch_size must be greater than 0".to_string(),
        ));
    }
    if placement_batch_size == 0 {
        return Err(AppError::Validation(
            "placement_batch_size must be greater than 0".to_string(),
        ));
    }
    if keyword_batch_size == 0 {
        return Err(AppError::Validation(
            "keyword_batch_size must be greater than 0".to_string(),
        ));
    }

    Ok(AppConfig {
        input,
        output,
        recursive,
        max_file_size_mb,
        page_cutoff,
        pdf_extract_workers,
        category_depth,
        taxonomy_mode,
        taxonomy_batch_size,
        placement_batch_size,
        placement_mode,
        rebuild,
        dry_run,
        llm_provider,
        llm_model,
        llm_base_url,
        api_key,
        keyword_batch_size,
        batch_start_delay_ms,
        verbose: verbosity >= 1,
        debug: verbosity >= 2,
        quiet: cli.quiet,
    })
}

fn normalize_verbosity(raw: u8) -> u8 {
    raw.min(2)
}

pub fn xdg_config_path() -> Option<PathBuf> {
    BaseDirs::new().map(|base| base.config_dir().join("sortyourpapers").join("config.toml"))
}

pub fn xdg_cache_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|base| base.cache_dir().join("sortyourpapers"))
}

pub fn init_xdg_config(force: bool) -> Result<PathBuf> {
    let Some(path) = xdg_config_path() else {
        return Err(AppError::Config(
            "could not resolve XDG config directory".to_string(),
        ));
    };

    write_default_config_at(&path, force)?;
    Ok(path)
}

pub fn default_config_toml() -> String {
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
    )
}

fn load_xdg_config() -> Result<FileConfig> {
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

fn parse_env_bool(key: &str) -> Result<Option<bool>> {
    match env::var(key) {
        Ok(v) => parse_bool(key, &v).map(Some),
        Err(_) => Ok(None),
    }
}

fn parse_env_u64(key: &str) -> Result<Option<u64>> {
    match env::var(key) {
        Ok(v) => v
            .parse::<u64>()
            .map(Some)
            .map_err(|_| AppError::Config(format!("{key} must be a positive integer"))),
        Err(_) => Ok(None),
    }
}

fn parse_env_u8(key: &str) -> Result<Option<u8>> {
    match env::var(key) {
        Ok(v) => v
            .parse::<u8>()
            .map(Some)
            .map_err(|_| AppError::Config(format!("{key} must be an integer 0-255"))),
        Err(_) => Ok(None),
    }
}

fn parse_env_usize(key: &str) -> Result<Option<usize>> {
    match env::var(key) {
        Ok(v) => v
            .parse::<usize>()
            .map(Some)
            .map_err(|_| AppError::Config(format!("{key} must be a positive integer"))),
        Err(_) => Ok(None),
    }
}

fn parse_env_provider(key: &str) -> Result<Option<LlmProvider>> {
    match env::var(key) {
        Ok(v) => match v.to_ascii_lowercase().as_str() {
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
        Ok(v) => match v.to_ascii_lowercase().as_str() {
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
        Ok(v) => match v.to_ascii_lowercase().as_str() {
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

fn write_default_config_at(path: &Path, force: bool) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use clap::Parser;
    use tempfile::tempdir;

    use super::{
        Cli, CliArgs, Commands, EnvConfig, FileConfig, resolve_from_sources,
        write_default_config_at,
    };
    use crate::models::{LlmProvider, PlacementMode, TaxonomyMode};
    use crate::pdf_extract::ExtractorMode;

    #[test]
    fn cli_overrides_env_and_file() {
        let cli = CliArgs::parse_from([
            "sortyourpapers",
            "--input",
            "/cli/input",
            "--output",
            "/cli/output",
            "--recursive",
            "--max-file-size-mb",
            "7",
            "--page-cutoff",
            "4",
            "--pdf-extract-workers",
            "6",
            "--category-depth",
            "3",
            "--taxonomy-mode",
            "batch-merge",
            "--taxonomy-batch-size",
            "6",
            "--placement-batch-size",
            "14",
            "--placement-mode",
            "allow-new",
            "--rebuild",
            "--apply",
            "--llm-provider",
            "openai",
            "--llm-model",
            "gpt-test",
            "--llm-base-url",
            "http://cli.example/v1",
            "--api-key",
            "cli-key",
            "--keyword-batch-size",
            "12",
            "-vv",
        ]);

        let env_cfg = EnvConfig {
            input: Some(PathBuf::from("/env/input")),
            output: Some(PathBuf::from("/env/output")),
            recursive: Some(false),
            max_file_size_mb: Some(100),
            page_cutoff: Some(10),
            pdf_extract_workers: Some(7),
            category_depth: Some(5),
            taxonomy_mode: Some(TaxonomyMode::BatchMerge),
            taxonomy_batch_size: Some(9),
            placement_batch_size: Some(18),
            placement_mode: Some(PlacementMode::ExistingOnly),
            rebuild: Some(false),
            llm_provider: Some(LlmProvider::Ollama),
            llm_model: Some("env-model".to_string()),
            llm_base_url: Some("http://env".to_string()),
            api_key: Some("env-key".to_string()),
            keyword_batch_size: Some(30),
            batch_start_delay_ms: Some(250),
        };

        let file_cfg = FileConfig {
            input: Some(PathBuf::from("/file/input")),
            output: Some(PathBuf::from("/file/output")),
            recursive: Some(false),
            max_file_size_mb: Some(200),
            page_cutoff: Some(20),
            pdf_extract_workers: Some(8),
            category_depth: Some(6),
            taxonomy_mode: Some(TaxonomyMode::BatchMerge),
            taxonomy_batch_size: Some(8),
            placement_batch_size: Some(16),
            placement_mode: Some(PlacementMode::ExistingOnly),
            rebuild: Some(false),
            llm_provider: Some(LlmProvider::Ollama),
            llm_model: Some("file-model".to_string()),
            llm_base_url: Some("http://file".to_string()),
            api_key: Some("file-key".to_string()),
            keyword_batch_size: Some(25),
            batch_start_delay_ms: Some(150),
        };

        let cfg = resolve_from_sources(cli, env_cfg, file_cfg).expect("config should resolve");

        assert_eq!(cfg.input, PathBuf::from("/cli/input"));
        assert_eq!(cfg.output, PathBuf::from("/cli/output"));
        assert!(cfg.recursive);
        assert_eq!(cfg.max_file_size_mb, 7);
        assert_eq!(cfg.page_cutoff, 4);
        assert_eq!(cfg.pdf_extract_workers, 6);
        assert_eq!(cfg.category_depth, 3);
        assert_eq!(cfg.taxonomy_mode, TaxonomyMode::BatchMerge);
        assert_eq!(cfg.taxonomy_batch_size, 6);
        assert_eq!(cfg.placement_batch_size, 14);
        assert_eq!(cfg.placement_mode, PlacementMode::AllowNew);
        assert!(cfg.rebuild);
        assert!(!cfg.dry_run);
        assert_eq!(cfg.llm_provider, LlmProvider::Openai);
        assert_eq!(cfg.llm_model, "gpt-test");
        assert_eq!(cfg.llm_base_url.as_deref(), Some("http://cli.example/v1"));
        assert_eq!(cfg.api_key.as_deref(), Some("cli-key"));
        assert_eq!(cfg.keyword_batch_size, 12);
        assert_eq!(cfg.batch_start_delay_ms, 250);
        assert!(cfg.verbose);
        assert!(cfg.debug);
    }

    #[test]
    fn init_writes_default_config() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");

        write_default_config_at(&path, false).expect("write default config");

        let raw = fs::read_to_string(path).expect("read config");
        assert!(raw.contains("max_file_size_mb = 16"));
        assert!(raw.contains("pdf_extract_workers = 8"));
        assert!(raw.contains("llm_provider = \"gemini\""));
        assert!(raw.contains("llm_model = \"gemini-3-flash-preview\""));
        assert!(raw.contains("taxonomy_mode = \"batch-merge\""));
        assert!(raw.contains("taxonomy_batch_size = 4"));
        assert!(raw.contains("placement_batch_size = 10"));
        assert!(raw.contains("keyword_batch_size = 20"));
        assert!(raw.contains("batch_start_delay_ms = 100"));
        assert!(!raw.contains("dry_run ="));
    }

    #[test]
    fn init_refuses_overwrite_without_force() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        fs::write(&path, "llm_provider=\"openai\"\n").expect("seed config");

        let result = write_default_config_at(&path, false);
        assert!(result.is_err());

        write_default_config_at(&path, true).expect("forced overwrite should work");
        let raw = fs::read_to_string(path).expect("read overwritten config");
        assert!(raw.contains("SortYourPapers default configuration"));
    }

    #[test]
    fn supports_gemini_provider_from_cli() {
        let cli = CliArgs::parse_from([
            "sortyourpapers",
            "--llm-provider",
            "gemini",
            "--llm-model",
            "gemini-2.0-flash",
        ]);

        let cfg =
            resolve_from_sources(cli, EnvConfig::default(), FileConfig::default()).expect("config");
        assert_eq!(cfg.llm_provider, LlmProvider::Gemini);
    }

    #[test]
    fn defaults_to_gemini_and_working_model_when_missing() {
        let cli = CliArgs::parse_from(["sortyourpapers"]);

        let cfg =
            resolve_from_sources(cli, EnvConfig::default(), FileConfig::default()).expect("config");

        assert_eq!(cfg.llm_provider, LlmProvider::Gemini);
        assert_eq!(cfg.llm_model, "gemini-3-flash-preview");
        assert_eq!(cfg.pdf_extract_workers, 8);
        assert_eq!(cfg.taxonomy_mode, TaxonomyMode::BatchMerge);
        assert_eq!(cfg.taxonomy_batch_size, 4);
        assert_eq!(cfg.placement_batch_size, 10);
        assert_eq!(cfg.keyword_batch_size, 20);
        assert_eq!(cfg.batch_start_delay_ms, 100);
        assert!(cfg.dry_run);
        assert!(!cfg.verbose);
        assert!(!cfg.debug);
    }

    #[test]
    fn supports_shorthand_flags() {
        let cli = CliArgs::parse_from([
            "sortyourpapers",
            "-i",
            "/tmp/in",
            "-o",
            "/tmp/out",
            "-r",
            "-s",
            "16",
            "-p",
            "4",
            "--pdf-extract-workers",
            "5",
            "-d",
            "3",
            "--taxonomy-mode",
            "batch-merge",
            "--taxonomy-batch-size",
            "5",
            "--placement-batch-size",
            "15",
            "-M",
            "allow-new",
            "-R",
            "-a",
            "-P",
            "gemini",
            "-m",
            "gemini-2.5-pro",
            "-u",
            "https://generativelanguage.googleapis.com/v1beta",
            "-k",
            "abc",
            "--keyword-batch-size",
            "64",
            "-vv",
        ]);

        let cfg =
            resolve_from_sources(cli, EnvConfig::default(), FileConfig::default()).expect("config");

        assert_eq!(cfg.input, PathBuf::from("/tmp/in"));
        assert_eq!(cfg.output, PathBuf::from("/tmp/out"));
        assert!(cfg.recursive);
        assert_eq!(cfg.max_file_size_mb, 16);
        assert_eq!(cfg.page_cutoff, 4);
        assert_eq!(cfg.pdf_extract_workers, 5);
        assert_eq!(cfg.category_depth, 3);
        assert_eq!(cfg.taxonomy_mode, TaxonomyMode::BatchMerge);
        assert_eq!(cfg.taxonomy_batch_size, 5);
        assert_eq!(cfg.placement_batch_size, 15);
        assert_eq!(cfg.placement_mode, PlacementMode::AllowNew);
        assert!(cfg.rebuild);
        assert!(!cfg.dry_run);
        assert!(cfg.llm_provider == LlmProvider::Gemini);
        assert_eq!(cfg.llm_model, "gemini-2.5-pro");
        assert_eq!(
            cfg.llm_base_url.as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta")
        );
        assert_eq!(cfg.api_key.as_deref(), Some("abc"));
        assert_eq!(cfg.keyword_batch_size, 64);
        assert_eq!(cfg.batch_start_delay_ms, 100);
        assert!(cfg.verbose);
        assert!(cfg.debug);
    }

    #[test]
    fn parses_extract_text_subcommand() {
        let cli = Cli::parse_from([
            "sortyourpapers",
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
    fn parses_legacy_lopdf_extractor_alias() {
        let cli = Cli::parse_from([
            "sortyourpapers",
            "extract-text",
            "--extractor",
            "lopdf",
            "/tmp/a.pdf",
        ]);

        match cli.command {
            Some(Commands::ExtractText(args)) => {
                assert_eq!(args.extractor, ExtractorMode::PdfOxide);
                assert_eq!(args.pdf_extract_workers, 8);
                assert_eq!(args.files.len(), 1);
            }
            _ => panic!("expected extract-text command"),
        }
    }

    #[test]
    fn parses_resume_subcommand() {
        let cli = Cli::parse_from(["sortyourpapers", "resume", "run-123"]);

        match cli.command {
            Some(Commands::Resume(args)) => {
                assert_eq!(args.run_id.as_deref(), Some("run-123"));
                assert!(!args.apply);
            }
            _ => panic!("expected resume command"),
        }
    }

    #[test]
    fn parses_resume_verbosity_override() {
        let cli = Cli::parse_from(["sortyourpapers", "resume", "--apply", "-vv", "run-123"]);

        match cli.command {
            Some(Commands::Resume(args)) => {
                assert_eq!(args.run_id.as_deref(), Some("run-123"));
                assert!(args.apply);
                assert_eq!(args.verbosity, 2);
            }
            _ => panic!("expected resume command"),
        }
    }
}
