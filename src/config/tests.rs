use std::{fs, path::PathBuf};

use clap::Parser;
use tempfile::tempdir;

use super::{
    Cli, CliArgs, Commands, EnvConfig, FileConfig, SessionCommands, resolve::resolve_from_sources,
    xdg::write_default_config_at,
};
use crate::models::{LlmProvider, PlacementMode, TaxonomyMode};
use crate::pdf_extract::ExtractorMode;
use crate::run_state::RunStage;

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
    assert_eq!(cfg.llm_provider, LlmProvider::Gemini);
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
fn parses_session_aliases() {
    let cli = Cli::parse_from(["sortyourpapers", "ses", "ls"]);

    match cli.command {
        Some(Commands::Session(args)) => match args.command {
            SessionCommands::List => {}
            _ => panic!("expected session list command"),
        },
        _ => panic!("expected session command"),
    }
}

#[test]
fn parses_session_remove_alias_with_multiple_run_ids() {
    let cli = Cli::parse_from(["sortyourpapers", "session", "rm", "run-1", "run-2"]);

    match cli.command {
        Some(Commands::Session(args)) => match args.command {
            SessionCommands::Remove(args) => {
                assert_eq!(args.run_ids, vec!["run-1", "run-2"]);
            }
            _ => panic!("expected session remove command"),
        },
        _ => panic!("expected session command"),
    }
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
