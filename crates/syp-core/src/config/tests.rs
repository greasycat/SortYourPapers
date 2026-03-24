use std::{fs, path::PathBuf};

use tempfile::tempdir;

use super::{
    ApiKeySource, AppConfig, EnvConfig, FileConfig,
    resolve::resolve_from_sources,
    xdg::{
        default_testset_cache_dir, shared_testset_cache_dir_from, write_default_config_at,
        write_saved_config_at,
    },
};
use crate::{
    inputs::RunOverrides,
    llm::LlmProvider,
    papers::placement::PlacementMode,
    papers::taxonomy::{TaxonomyAssistance, TaxonomyMode},
};

#[test]
fn overrides_beat_env_and_file_sources() {
    let overrides = RunOverrides {
        input: Some(PathBuf::from("/cli/input")),
        output: Some(PathBuf::from("/cli/output")),
        recursive: Some(true),
        max_file_size_mb: Some(7),
        page_cutoff: Some(4),
        pdf_extract_workers: Some(6),
        category_depth: Some(3),
        taxonomy_mode: Some(TaxonomyMode::BatchMerge),
        taxonomy_assistance: Some(TaxonomyAssistance::EmbeddingGuided),
        taxonomy_batch_size: Some(6),
        reference_manifest_path: Some(PathBuf::from("/cli/references.toml")),
        reference_top_k: Some(7),
        use_current_folder_tree: Some(true),
        placement_batch_size: Some(14),
        placement_mode: Some(PlacementMode::AllowNew),
        rebuild: Some(true),
        apply: true,
        llm_provider: Some(LlmProvider::Openai),
        llm_model: Some("gpt-test".to_string()),
        llm_base_url: Some("http://cli.example/v1".to_string()),
        api_key: Some("cli-key".to_string()),
        api_key_command: None,
        api_key_env: None,
        embedding_provider: Some(LlmProvider::Openai),
        embedding_model: Some("text-embedding-3-small".to_string()),
        embedding_base_url: Some("http://embed.example/v1".to_string()),
        embedding_api_key: Some("embed-key".to_string()),
        embedding_api_key_command: None,
        embedding_api_key_env: None,
        keyword_batch_size: Some(12),
        subcategories_suggestion_number: Some(9),
        verbosity: 2,
        quiet: false,
    };

    let env_cfg = EnvConfig {
        input: Some(PathBuf::from("/env/input")),
        output: Some(PathBuf::from("/env/output")),
        recursive: Some(false),
        max_file_size_mb: Some(100),
        page_cutoff: Some(10),
        pdf_extract_workers: Some(7),
        category_depth: Some(5),
        taxonomy_mode: Some(TaxonomyMode::BatchMerge),
        taxonomy_assistance: Some(TaxonomyAssistance::LlmOnly),
        taxonomy_batch_size: Some(9),
        reference_manifest_path: Some(PathBuf::from("/env/references.toml")),
        reference_top_k: Some(6),
        use_current_folder_tree: Some(false),
        placement_batch_size: Some(18),
        placement_mode: Some(PlacementMode::ExistingOnly),
        rebuild: Some(false),
        llm_provider: Some(LlmProvider::Ollama),
        llm_model: Some("env-model".to_string()),
        llm_base_url: Some("http://env".to_string()),
        api_key: Some(ApiKeySource::Text("env-key".to_string())),
        embedding_provider: Some(LlmProvider::Gemini),
        embedding_model: Some("text-embedding-004".to_string()),
        embedding_base_url: Some("http://env-embed".to_string()),
        embedding_api_key: Some(ApiKeySource::Text("env-embed-key".to_string())),
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
        taxonomy_assistance: Some(TaxonomyAssistance::LlmOnly),
        taxonomy_batch_size: Some(8),
        reference_manifest_path: Some(PathBuf::from("/file/references.toml")),
        reference_top_k: Some(5),
        use_current_folder_tree: Some(false),
        placement_batch_size: Some(16),
        placement_mode: Some(PlacementMode::ExistingOnly),
        rebuild: Some(false),
        llm_provider: Some(LlmProvider::Ollama),
        llm_model: Some("file-model".to_string()),
        llm_base_url: Some("http://file".to_string()),
        api_key: Some(ApiKeySource::Text("file-key".to_string())),
        embedding_provider: Some(LlmProvider::Ollama),
        embedding_model: Some("nomic-embed-text".to_string()),
        embedding_base_url: Some("http://file-embed".to_string()),
        embedding_api_key: Some(ApiKeySource::Text("file-embed-key".to_string())),
        keyword_batch_size: Some(25),
        batch_start_delay_ms: Some(150),
        subcategories_suggestion_number: Some(7),
    };

    let cfg = resolve_from_sources(overrides, env_cfg, file_cfg).expect("config should resolve");

    assert_eq!(cfg.input, PathBuf::from("/cli/input"));
    assert_eq!(cfg.output, PathBuf::from("/cli/output"));
    assert!(cfg.recursive);
    assert_eq!(cfg.max_file_size_mb, 7);
    assert_eq!(cfg.page_cutoff, 4);
    assert_eq!(cfg.pdf_extract_workers, 6);
    assert_eq!(cfg.category_depth, 3);
    assert_eq!(cfg.taxonomy_assistance, TaxonomyAssistance::EmbeddingGuided);
    assert_eq!(cfg.taxonomy_batch_size, 6);
    assert_eq!(
        cfg.reference_manifest_path,
        PathBuf::from("/cli/references.toml")
    );
    assert_eq!(cfg.reference_top_k, 7);
    assert!(cfg.use_current_folder_tree);
    assert_eq!(cfg.placement_batch_size, 14);
    assert_eq!(cfg.placement_mode, PlacementMode::AllowNew);
    assert!(cfg.rebuild);
    assert!(!cfg.dry_run);
    assert_eq!(cfg.llm_provider, LlmProvider::Openai);
    assert_eq!(cfg.llm_model, "gpt-test");
    assert_eq!(cfg.llm_base_url.as_deref(), Some("http://cli.example/v1"));
    assert_eq!(cfg.api_key, Some(ApiKeySource::Text("cli-key".to_string())));
    assert_eq!(cfg.embedding_provider, LlmProvider::Openai);
    assert_eq!(cfg.embedding_model, "text-embedding-3-small");
    assert_eq!(
        cfg.embedding_base_url.as_deref(),
        Some("http://embed.example/v1")
    );
    assert_eq!(
        cfg.embedding_api_key,
        Some(ApiKeySource::Text("embed-key".to_string()))
    );
    assert_eq!(cfg.keyword_batch_size, 12);
    assert_eq!(cfg.batch_start_delay_ms, 250);
    assert_eq!(cfg.subcategories_suggestion_number, 9);
    assert!(cfg.verbose);
    assert!(cfg.debug);
}

#[test]
fn defaults_to_gemini_when_no_sources_provide_values() {
    let cfg = resolve_from_sources(
        RunOverrides::default(),
        EnvConfig::default(),
        FileConfig::default(),
    )
    .expect("config");

    assert_eq!(cfg.llm_provider, LlmProvider::Gemini);
    assert_eq!(cfg.llm_model, "gemini-3-flash-preview");
    assert_eq!(cfg.taxonomy_assistance, TaxonomyAssistance::LlmOnly);
    assert_eq!(
        cfg.reference_manifest_path,
        PathBuf::from("assets/testsets/scijudgebench-diverse.toml")
    );
    assert_eq!(cfg.reference_top_k, 5);
    assert_eq!(cfg.pdf_extract_workers, 8);
    assert_eq!(cfg.taxonomy_mode, TaxonomyMode::BatchMerge);
    assert_eq!(cfg.taxonomy_batch_size, 4);
    assert!(!cfg.use_current_folder_tree);
    assert_eq!(cfg.placement_batch_size, 10);
    assert_eq!(cfg.keyword_batch_size, 20);
    assert_eq!(cfg.embedding_provider, LlmProvider::Gemini);
    assert_eq!(cfg.embedding_model, "text-embedding-004");
    assert_eq!(cfg.batch_start_delay_ms, 100);
    assert_eq!(cfg.subcategories_suggestion_number, 5);
    assert!(cfg.dry_run);
    assert!(!cfg.verbose);
    assert!(!cfg.debug);
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
        taxonomy_assistance: TaxonomyAssistance::EmbeddingGuided,
        taxonomy_batch_size: 9,
        reference_manifest_path: PathBuf::from("/references/demo.toml"),
        reference_top_k: 8,
        use_current_folder_tree: true,
        placement_batch_size: 11,
        placement_mode: PlacementMode::AllowNew,
        rebuild: true,
        dry_run: false,
        llm_provider: LlmProvider::Openai,
        llm_model: "gpt-test".to_string(),
        llm_base_url: Some("http://localhost:1234/v1".to_string()),
        api_key: Some(ApiKeySource::Env("OPENAI_API_KEY".to_string())),
        embedding_provider: LlmProvider::Openai,
        embedding_model: "text-embedding-3-small".to_string(),
        embedding_base_url: Some("http://localhost:1234/embeddings".to_string()),
        embedding_api_key: Some(ApiKeySource::Env("OPENAI_EMBED_API_KEY".to_string())),
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
    assert!(raw.contains("placement_mode = \"allow-new\""));
    assert!(raw.contains("llm_provider = \"openai\""));
    assert!(raw.contains("taxonomy_assistance = \"embedding-guided\""));
    assert!(raw.contains("embedding_model = \"text-embedding-3-small\""));
    assert!(raw.contains("batch_start_delay_ms = 250"));
    assert!(!raw.contains("dry_run ="));
    assert!(!raw.contains("quiet ="));
}

#[test]
fn resolved_api_key_reads_from_env_source() {
    let expected = std::env::var("PATH").expect("PATH should exist for tests");
    let config = sample_config(Some(ApiKeySource::Env("PATH".to_string())));

    let resolved = config
        .resolved_api_key()
        .expect("env source should resolve successfully");

    assert_eq!(resolved, Some(expected));
}

#[test]
fn resolved_api_key_runs_command_source() {
    let config = sample_config(Some(ApiKeySource::Command("printf 'cmd-key'".to_string())));

    let resolved = config
        .resolved_api_key()
        .expect("command source should resolve successfully");

    assert_eq!(resolved, Some("cmd-key".to_string()));
}

#[test]
fn shared_testset_cache_dir_uses_dev_toml_relative_path() {
    let dir = tempdir().expect("tempdir");
    let nested = dir.path().join("crates").join("syp");
    fs::create_dir_all(&nested).expect("mkdir");
    fs::write(
        dir.path().join("dev.toml"),
        "[testsets]\ncache_dir = \".cache/sortyourpapers/testsets\"\n",
    )
    .expect("write dev config");

    let resolved = shared_testset_cache_dir_from(&nested)
        .expect("resolve shared cache")
        .expect("dev.toml path");

    assert_eq!(
        resolved,
        dir.path()
            .join(".cache")
            .join("sortyourpapers")
            .join("testsets")
    );
}

#[test]
fn shared_testset_cache_dir_falls_back_to_xdg_layout() {
    let resolved = default_testset_cache_dir().expect("resolve fallback testset cache");
    assert!(resolved.ends_with("sortyourpapers/testsets"));
}

fn sample_config(api_key: Option<ApiKeySource>) -> AppConfig {
    AppConfig {
        input: PathBuf::from("/papers"),
        output: PathBuf::from("/sorted"),
        recursive: false,
        max_file_size_mb: 16,
        page_cutoff: 1,
        pdf_extract_workers: 8,
        category_depth: 2,
        taxonomy_mode: TaxonomyMode::BatchMerge,
        taxonomy_assistance: TaxonomyAssistance::LlmOnly,
        taxonomy_batch_size: 4,
        reference_manifest_path: PathBuf::from("assets/testsets/scijudgebench-diverse.toml"),
        reference_top_k: 5,
        use_current_folder_tree: false,
        placement_batch_size: 10,
        placement_mode: PlacementMode::ExistingOnly,
        rebuild: false,
        dry_run: true,
        llm_provider: LlmProvider::Gemini,
        llm_model: "gemini-3-flash-preview".to_string(),
        llm_base_url: None,
        api_key,
        embedding_provider: LlmProvider::Gemini,
        embedding_model: "text-embedding-004".to_string(),
        embedding_base_url: None,
        embedding_api_key: None,
        keyword_batch_size: 20,
        batch_start_delay_ms: 100,
        subcategories_suggestion_number: 5,
        verbose: false,
        debug: false,
        quiet: false,
    }
}
