use std::{fs, path::PathBuf};

use clap::Parser;
use tempfile::tempdir;

use super::{
    ApiKeySource, AppConfig, Cli, CliArgs, Commands, EnvConfig, FileConfig, SessionCommands,
    TuiPreferences,
    resolve::resolve_from_sources,
    xdg::{
        load_tui_preferences_from_path, write_default_config_at, write_saved_config_at,
        write_tui_preferences_at,
    },
};
use crate::llm::LlmProvider;
use crate::papers::extract::ExtractorMode;
use crate::papers::placement::PlacementMode;
use crate::papers::taxonomy::TaxonomyMode;
use crate::session::RunStage;
use crate::tui::theme::UiThemeName;

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
        "--use-current-folder-tree",
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
        use_current_folder_tree: Some(false),
        placement_batch_size: Some(18),
        placement_mode: Some(PlacementMode::ExistingOnly),
        rebuild: Some(false),
        llm_provider: Some(LlmProvider::Ollama),
        llm_model: Some("env-model".to_string()),
        llm_base_url: Some("http://env".to_string()),
        api_key: Some(ApiKeySource::Text("env-key".to_string())),
        keyword_batch_size: Some(30),
        batch_start_delay_ms: Some(250),
        subcategories_suggestion_number: Some(9),
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
        use_current_folder_tree: Some(false),
        placement_batch_size: Some(16),
        placement_mode: Some(PlacementMode::ExistingOnly),
        rebuild: Some(false),
        llm_provider: Some(LlmProvider::Ollama),
        llm_model: Some("file-model".to_string()),
        llm_base_url: Some("http://file".to_string()),
        api_key: Some(ApiKeySource::Text("file-key".to_string())),
        keyword_batch_size: Some(25),
        batch_start_delay_ms: Some(150),
        subcategories_suggestion_number: Some(7),
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
    assert!(cfg.use_current_folder_tree);
    assert_eq!(cfg.placement_batch_size, 14);
    assert_eq!(cfg.placement_mode, PlacementMode::AllowNew);
    assert!(cfg.rebuild);
    assert!(!cfg.dry_run);
    assert_eq!(cfg.llm_provider, LlmProvider::Openai);
    assert_eq!(cfg.llm_model, "gpt-test");
    assert_eq!(cfg.llm_base_url.as_deref(), Some("http://cli.example/v1"));
    assert_eq!(cfg.api_key, Some(ApiKeySource::Text("cli-key".to_string())));
    assert_eq!(cfg.keyword_batch_size, 12);
    assert_eq!(cfg.batch_start_delay_ms, 250);
    assert_eq!(cfg.subcategories_suggestion_number, 9);
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
    assert!(raw.contains("use_current_folder_tree = false"));
    assert!(raw.contains("placement_batch_size = 10"));
    assert!(raw.contains("keyword_batch_size = 20"));
    assert!(raw.contains("batch_start_delay_ms = 100"));
    assert!(raw.contains("subcategories_suggestion_number = 5"));
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
fn save_writes_current_config_values() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let config = AppConfig {
        input: PathBuf::from("/papers"),
        output: PathBuf::from("/sorted"),
        recursive: true,
        max_file_size_mb: 32,
        page_cutoff: 3,
        pdf_extract_workers: 6,
        category_depth: 4,
        taxonomy_mode: TaxonomyMode::Global,
        taxonomy_batch_size: 9,
        use_current_folder_tree: true,
        placement_batch_size: 11,
        placement_mode: PlacementMode::AllowNew,
        rebuild: true,
        dry_run: false,
        llm_provider: LlmProvider::Openai,
        llm_model: "gpt-test".to_string(),
        llm_base_url: Some("http://localhost:1234/v1".to_string()),
        api_key: Some(ApiKeySource::Env("OPENAI_API_KEY".to_string())),
        keyword_batch_size: 21,
        batch_start_delay_ms: 250,
        subcategories_suggestion_number: 7,
        verbose: true,
        debug: false,
        quiet: true,
    };

    write_saved_config_at(&path, &config).expect("save config");

    let raw = fs::read_to_string(path).expect("read config");
    assert!(raw.contains("SortYourPapers saved configuration"));
    assert!(raw.contains("input = \"/papers\""));
    assert!(raw.contains("output = \"/sorted\""));
    assert!(raw.contains("recursive = true"));
    assert!(raw.contains("taxonomy_mode = \"global\""));
    assert!(raw.contains("use_current_folder_tree = true"));
    assert!(raw.contains("placement_mode = \"allow-new\""));
    assert!(raw.contains("llm_provider = \"openai\""));
    assert!(raw.contains("llm_model = \"gpt-test\""));
    assert!(raw.contains("llm_base_url = \"http://localhost:1234/v1\""));
    assert!(raw.contains("source = \"env\""));
    assert!(raw.contains("value = \"OPENAI_API_KEY\""));
    assert!(raw.contains("batch_start_delay_ms = 250"));
    assert!(!raw.contains("dry_run ="));
    assert!(!raw.contains("quiet ="));
}

#[test]
fn resolved_api_key_reads_from_env_source() {
    let expected = std::env::var("PATH").expect("PATH should exist for tests");
    let config = AppConfig {
        input: PathBuf::from("/papers"),
        output: PathBuf::from("/sorted"),
        recursive: false,
        max_file_size_mb: 16,
        page_cutoff: 1,
        pdf_extract_workers: 8,
        category_depth: 2,
        taxonomy_mode: TaxonomyMode::BatchMerge,
        taxonomy_batch_size: 4,
        use_current_folder_tree: false,
        placement_batch_size: 10,
        placement_mode: PlacementMode::ExistingOnly,
        rebuild: false,
        dry_run: true,
        llm_provider: LlmProvider::Gemini,
        llm_model: "gemini-3-flash-preview".to_string(),
        llm_base_url: None,
        api_key: Some(ApiKeySource::Env("PATH".to_string())),
        keyword_batch_size: 20,
        batch_start_delay_ms: 100,
        subcategories_suggestion_number: 5,
        verbose: false,
        debug: false,
        quiet: false,
    };

    let resolved = config
        .resolved_api_key()
        .expect("env source should resolve successfully");

    assert_eq!(resolved, Some(expected));
}

#[test]
fn resolved_api_key_runs_command_source() {
    let config = AppConfig {
        input: PathBuf::from("/papers"),
        output: PathBuf::from("/sorted"),
        recursive: false,
        max_file_size_mb: 16,
        page_cutoff: 1,
        pdf_extract_workers: 8,
        category_depth: 2,
        taxonomy_mode: TaxonomyMode::BatchMerge,
        taxonomy_batch_size: 4,
        use_current_folder_tree: false,
        placement_batch_size: 10,
        placement_mode: PlacementMode::ExistingOnly,
        rebuild: false,
        dry_run: true,
        llm_provider: LlmProvider::Gemini,
        llm_model: "gemini-3-flash-preview".to_string(),
        llm_base_url: None,
        api_key: Some(ApiKeySource::Command("printf 'cmd-key'".to_string())),
        keyword_batch_size: 20,
        batch_start_delay_ms: 100,
        subcategories_suggestion_number: 5,
        verbose: false,
        debug: false,
        quiet: false,
    };

    let resolved = config
        .resolved_api_key()
        .expect("command source should resolve successfully");

    assert_eq!(resolved, Some("cmd-key".to_string()));
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
    assert!(!cfg.use_current_folder_tree);
    assert_eq!(cfg.placement_batch_size, 10);
    assert_eq!(cfg.keyword_batch_size, 20);
    assert_eq!(cfg.batch_start_delay_ms, 100);
    assert_eq!(cfg.subcategories_suggestion_number, 5);
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
        "--use-current-folder-tree",
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
        "--subcategories-suggestion-number",
        "11",
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
    assert!(cfg.use_current_folder_tree);
    assert_eq!(cfg.placement_batch_size, 15);
    assert_eq!(cfg.placement_mode, PlacementMode::AllowNew);
    assert!(cfg.rebuild);
    assert!(!cfg.dry_run);
    assert_eq!(cfg.llm_provider, LlmProvider::Gemini);
    assert_eq!(cfg.llm_model, "gemini-2.5-pro");
    assert_eq!(
        cfg.llm_base_url.as_deref(),
        Some("https://generativelanguage.googleapis.com/v1beta")
    );
    assert_eq!(cfg.api_key, Some(ApiKeySource::Text("abc".to_string())));
    assert_eq!(cfg.keyword_batch_size, 64);
    assert_eq!(cfg.batch_start_delay_ms, 100);
    assert_eq!(cfg.subcategories_suggestion_number, 11);
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
fn parses_session_resume_subcommand() {
    let cli = Cli::parse_from(["sortyourpapers", "session", "resume", "run-123"]);

    match cli.command {
        Some(Commands::Session(args)) => match args.command {
            SessionCommands::Resume(args) => {
                assert_eq!(args.run_id.as_deref(), Some("run-123"));
                assert!(!args.apply);
            }
            _ => panic!("expected session resume command"),
        },
        _ => panic!("expected session command"),
    }
}

#[test]
fn parses_session_review_subcommand() {
    let cli = Cli::parse_from(["sortyourpapers", "session", "review", "run-123"]);

    match cli.command {
        Some(Commands::Session(args)) => match args.command {
            SessionCommands::Review(args) => {
                assert_eq!(args.run_id.as_deref(), Some("run-123"));
            }
            _ => panic!("expected session review command"),
        },
        _ => panic!("expected session command"),
    }
}

#[test]
fn parses_session_resume_verbosity_override() {
    let cli = Cli::parse_from([
        "sortyourpapers",
        "session",
        "resume",
        "--apply",
        "-vv",
        "run-123",
    ]);

    match cli.command {
        Some(Commands::Session(args)) => match args.command {
            SessionCommands::Resume(args) => {
                assert_eq!(args.run_id.as_deref(), Some("run-123"));
                assert!(args.apply);
                assert_eq!(args.verbosity, 2);
            }
            _ => panic!("expected session resume command"),
        },
        _ => panic!("expected session command"),
    }
}

#[test]
fn parses_session_rerun_subcommand_with_stage() {
    let cli = Cli::parse_from([
        "sortyourpapers",
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
fn rejects_completed_rerun_stage() {
    let err = Cli::try_parse_from([
        "sortyourpapers",
        "session",
        "rerun",
        "--stage",
        "completed",
        "run-123",
    ])
    .expect_err("completed should not be a valid rerun stage");

    assert!(err.to_string().contains("invalid value"));
}

#[test]
fn rejects_bare_session_command() {
    let err = Cli::try_parse_from(["sortyourpapers", "session"])
        .expect_err("session should require a subcommand");

    assert!(err.to_string().contains("subcommand"));
}

#[test]
fn rejects_legacy_resume_command() {
    let err = Cli::try_parse_from(["sortyourpapers", "resume", "run-123"])
        .expect_err("legacy resume command should be rejected");

    assert!(err.to_string().contains("unrecognized"));
}

#[test]
fn tui_preferences_default_to_dark_when_missing() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("tui.toml");

    let loaded = load_tui_preferences_from_path(&path).expect("load missing prefs");

    assert_eq!(loaded.theme, UiThemeName::Dark);
}

#[test]
fn tui_preferences_round_trip_light_theme() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("tui.toml");
    let prefs = TuiPreferences {
        theme: UiThemeName::Light,
    };

    write_tui_preferences_at(&path, &prefs).expect("write prefs");
    let loaded = load_tui_preferences_from_path(&path).expect("load prefs");

    assert_eq!(loaded, prefs);
}

#[test]
fn invalid_tui_preferences_fall_back_to_dark() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("tui.toml");
    fs::write(&path, "theme = \"nope\"\n").expect("write invalid prefs");

    let loaded = load_tui_preferences_from_path(&path).expect("load prefs");

    assert_eq!(loaded.theme, UiThemeName::Dark);
}
