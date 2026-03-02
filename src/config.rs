use std::{
    env, fs,
    path::{Path, PathBuf},
};

use clap::{ArgAction, Args, Parser, Subcommand};
use directories::BaseDirs;
use serde::Deserialize;

use crate::{
    error::{AppError, Result},
    models::{AppConfig, LlmProvider, PlacementMode},
    pdf_extract::ExtractorMode,
};

const DEFAULT_INPUT: &str = ".";
const DEFAULT_OUTPUT: &str = "./sorted";
const DEFAULT_MAX_FILE_SIZE_MB: u64 = 8;
const DEFAULT_PAGE_CUTOFF: u8 = 1;
const DEFAULT_CATEGORY_DEPTH: u8 = 2;
const DEFAULT_RECURSIVE: bool = false;
const DEFAULT_REBUILD: bool = false;
const DEFAULT_DRY_RUN: bool = true;
const DEFAULT_LLM_PROVIDER: LlmProvider = LlmProvider::Gemini;
const DEFAULT_LLM_MODEL: &str = "gemini-3-flash-preview";
const DEFAULT_KEYWORD_BATCH_SIZE: usize = 50;

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

    #[arg(long, action = ArgAction::SetTrue)]
    pub debug: bool,
}

#[derive(Debug, Parser)]
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

    #[arg(short = 'd', long)]
    pub category_depth: Option<u8>,

    #[arg(short = 'M', long)]
    pub placement_mode: Option<PlacementMode>,

    #[arg(short = 'R', long, num_args = 0..=1, default_missing_value = "true")]
    pub rebuild: Option<bool>,

    #[arg(short = 'n', long, num_args = 0..=1, default_missing_value = "true")]
    pub dry_run: Option<bool>,

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

    #[arg(long, action = ArgAction::SetTrue)]
    pub debug: bool,
}

#[derive(Debug, Default, Deserialize, Clone)]
struct FileConfig {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    recursive: Option<bool>,
    max_file_size_mb: Option<u64>,
    page_cutoff: Option<u8>,
    category_depth: Option<u8>,
    placement_mode: Option<PlacementMode>,
    rebuild: Option<bool>,
    dry_run: Option<bool>,
    llm_provider: Option<LlmProvider>,
    llm_model: Option<String>,
    llm_base_url: Option<String>,
    api_key: Option<String>,
    keyword_batch_size: Option<usize>,
}

#[derive(Debug, Default)]
struct EnvConfig {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    recursive: Option<bool>,
    max_file_size_mb: Option<u64>,
    page_cutoff: Option<u8>,
    category_depth: Option<u8>,
    placement_mode: Option<PlacementMode>,
    rebuild: Option<bool>,
    dry_run: Option<bool>,
    llm_provider: Option<LlmProvider>,
    llm_model: Option<String>,
    llm_base_url: Option<String>,
    api_key: Option<String>,
    keyword_batch_size: Option<usize>,
}

impl EnvConfig {
    fn from_env() -> Result<Self> {
        Ok(Self {
            input: env::var("SYP_INPUT").ok().map(PathBuf::from),
            output: env::var("SYP_OUTPUT").ok().map(PathBuf::from),
            recursive: parse_env_bool("SYP_RECURSIVE")?,
            max_file_size_mb: parse_env_u64("SYP_MAX_FILE_SIZE_MB")?,
            page_cutoff: parse_env_u8("SYP_PAGE_CUTOFF")?,
            category_depth: parse_env_u8("SYP_CATEGORY_DEPTH")?,
            placement_mode: parse_env_placement_mode("SYP_PLACEMENT_MODE")?,
            rebuild: parse_env_bool("SYP_REBUILD")?,
            dry_run: parse_env_bool("SYP_DRY_RUN")?,
            llm_provider: parse_env_provider("SYP_LLM_PROVIDER")?,
            llm_model: env::var("SYP_LLM_MODEL").ok(),
            llm_base_url: env::var("SYP_LLM_BASE_URL").ok(),
            api_key: env::var("SYP_API_KEY").ok(),
            keyword_batch_size: parse_env_usize("SYP_KEYWORD_BATCH_SIZE")?,
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

    let category_depth = cli
        .category_depth
        .or(env_cfg.category_depth)
        .or(file_cfg.category_depth)
        .unwrap_or(DEFAULT_CATEGORY_DEPTH);

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

    let dry_run = if cli.apply {
        false
    } else {
        cli.dry_run
            .or(env_cfg.dry_run)
            .or(file_cfg.dry_run)
            .unwrap_or(DEFAULT_DRY_RUN)
    };

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
    if category_depth == 0 {
        return Err(AppError::Validation(
            "category_depth must be greater than 0".to_string(),
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
        category_depth,
        placement_mode,
        rebuild,
        dry_run,
        llm_provider,
        llm_model,
        llm_base_url,
        api_key,
        keyword_batch_size,
        debug: cli.debug,
    })
}

pub fn xdg_config_path() -> Option<PathBuf> {
    BaseDirs::new().map(|base| base.config_dir().join("sortyourpapers").join("config.toml"))
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
            "category_depth = {category_depth}\n",
            "placement_mode = \"existing-only\"\n",
            "rebuild = {rebuild}\n",
            "dry_run = {dry_run}\n",
            "\n",
            "# Default LLM settings\n",
            "llm_provider = \"gemini\"\n",
            "llm_model = \"gemini-2.5-flash\"\n",
            "keyword_batch_size = {keyword_batch_size}\n",
            "# llm_base_url = \"https://generativelanguage.googleapis.com/v1beta\"\n",
            "# api_key = \"\"\n"
        ),
        input = DEFAULT_INPUT,
        output = DEFAULT_OUTPUT,
        recursive = DEFAULT_RECURSIVE,
        max_file_size_mb = DEFAULT_MAX_FILE_SIZE_MB,
        page_cutoff = DEFAULT_PAGE_CUTOFF,
        category_depth = DEFAULT_CATEGORY_DEPTH,
        rebuild = DEFAULT_REBUILD,
        dry_run = DEFAULT_DRY_RUN,
        keyword_batch_size = DEFAULT_KEYWORD_BATCH_SIZE,
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
    use crate::models::{LlmProvider, PlacementMode};
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
            "--category-depth",
            "3",
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
            "--debug",
        ]);

        let env_cfg = EnvConfig {
            input: Some(PathBuf::from("/env/input")),
            output: Some(PathBuf::from("/env/output")),
            recursive: Some(false),
            max_file_size_mb: Some(100),
            page_cutoff: Some(10),
            category_depth: Some(5),
            placement_mode: Some(PlacementMode::ExistingOnly),
            rebuild: Some(false),
            dry_run: Some(true),
            llm_provider: Some(LlmProvider::Ollama),
            llm_model: Some("env-model".to_string()),
            llm_base_url: Some("http://env".to_string()),
            api_key: Some("env-key".to_string()),
            keyword_batch_size: Some(30),
        };

        let file_cfg = FileConfig {
            input: Some(PathBuf::from("/file/input")),
            output: Some(PathBuf::from("/file/output")),
            recursive: Some(false),
            max_file_size_mb: Some(200),
            page_cutoff: Some(20),
            category_depth: Some(6),
            placement_mode: Some(PlacementMode::ExistingOnly),
            rebuild: Some(false),
            dry_run: Some(true),
            llm_provider: Some(LlmProvider::Ollama),
            llm_model: Some("file-model".to_string()),
            llm_base_url: Some("http://file".to_string()),
            api_key: Some("file-key".to_string()),
            keyword_batch_size: Some(25),
        };

        let cfg = resolve_from_sources(cli, env_cfg, file_cfg).expect("config should resolve");

        assert_eq!(cfg.input, PathBuf::from("/cli/input"));
        assert_eq!(cfg.output, PathBuf::from("/cli/output"));
        assert!(cfg.recursive);
        assert_eq!(cfg.max_file_size_mb, 7);
        assert_eq!(cfg.page_cutoff, 4);
        assert_eq!(cfg.category_depth, 3);
        assert_eq!(cfg.placement_mode, PlacementMode::AllowNew);
        assert!(cfg.rebuild);
        assert!(!cfg.dry_run);
        assert_eq!(cfg.llm_provider, LlmProvider::Openai);
        assert_eq!(cfg.llm_model, "gpt-test");
        assert_eq!(cfg.llm_base_url.as_deref(), Some("http://cli.example/v1"));
        assert_eq!(cfg.api_key.as_deref(), Some("cli-key"));
        assert_eq!(cfg.keyword_batch_size, 12);
        assert!(cfg.debug);
    }

    #[test]
    fn init_writes_default_config() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");

        write_default_config_at(&path, false).expect("write default config");

        let raw = fs::read_to_string(path).expect("read config");
        assert!(raw.contains("max_file_size_mb = 8"));
        assert!(raw.contains("llm_provider = \"gemini\""));
        assert!(raw.contains("llm_model = \"gemini-2.5-flash\""));
        assert!(raw.contains("keyword_batch_size = 50"));
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
        assert_eq!(cfg.keyword_batch_size, 50);
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
            "-d",
            "3",
            "-M",
            "allow-new",
            "-R",
            "-n",
            "false",
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
            "--debug",
        ]);

        let cfg =
            resolve_from_sources(cli, EnvConfig::default(), FileConfig::default()).expect("config");

        assert_eq!(cfg.input, PathBuf::from("/tmp/in"));
        assert_eq!(cfg.output, PathBuf::from("/tmp/out"));
        assert!(cfg.recursive);
        assert_eq!(cfg.max_file_size_mb, 16);
        assert_eq!(cfg.page_cutoff, 4);
        assert_eq!(cfg.category_depth, 3);
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
            "lopdf",
            "--debug",
            "/tmp/a.pdf",
            "/tmp/b.pdf",
        ]);

        match cli.command {
            Some(Commands::ExtractText(args)) => {
                assert_eq!(args.page_cutoff, 2);
                assert_eq!(args.extractor, ExtractorMode::Lopdf);
                assert!(args.debug);
                assert_eq!(args.files.len(), 2);
            }
            _ => panic!("expected extract-text command"),
        }
    }
}
