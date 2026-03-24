use std::path::PathBuf;

use crate::{
    config::{ApiKeySource, AppConfig},
    defaults::{
        DEFAULT_BATCH_START_DELAY_MS, DEFAULT_CATEGORY_DEPTH, DEFAULT_GEMINI_EMBEDDING_MODEL,
        DEFAULT_INPUT, DEFAULT_KEYWORD_BATCH_SIZE, DEFAULT_LLM_MODEL, DEFAULT_LLM_PROVIDER,
        DEFAULT_MAX_FILE_SIZE_MB, DEFAULT_OPENAI_EMBEDDING_MODEL, DEFAULT_OUTPUT,
        DEFAULT_PAGE_CUTOFF, DEFAULT_PDF_EXTRACT_WORKERS, DEFAULT_PLACEMENT_BATCH_SIZE,
        DEFAULT_REBUILD, DEFAULT_RECURSIVE, DEFAULT_REFERENCE_MANIFEST_PATH,
        DEFAULT_REFERENCE_TOP_K, DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER,
        DEFAULT_TAXONOMY_ASSISTANCE, DEFAULT_TAXONOMY_BATCH_SIZE, DEFAULT_USE_CURRENT_FOLDER_TREE,
    },
    error::{AppError, Result},
    inputs::RunOverrides,
};

use super::{EnvConfig, FileConfig};

#[allow(clippy::too_many_lines)]
pub(super) fn resolve_from_sources(
    cli: RunOverrides,
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

    let taxonomy_assistance = cli
        .taxonomy_assistance
        .or(env_cfg.taxonomy_assistance)
        .or(file_cfg.taxonomy_assistance)
        .unwrap_or(DEFAULT_TAXONOMY_ASSISTANCE);

    let taxonomy_batch_size = cli
        .taxonomy_batch_size
        .or(env_cfg.taxonomy_batch_size)
        .or(file_cfg.taxonomy_batch_size)
        .unwrap_or(DEFAULT_TAXONOMY_BATCH_SIZE);

    let reference_manifest_path = cli
        .reference_manifest_path
        .or(env_cfg.reference_manifest_path)
        .or(file_cfg.reference_manifest_path)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_REFERENCE_MANIFEST_PATH));

    let reference_top_k = cli
        .reference_top_k
        .or(env_cfg.reference_top_k)
        .or(file_cfg.reference_top_k)
        .unwrap_or(DEFAULT_REFERENCE_TOP_K);

    let use_current_folder_tree = cli
        .use_current_folder_tree
        .or(env_cfg.use_current_folder_tree)
        .or(file_cfg.use_current_folder_tree)
        .unwrap_or(DEFAULT_USE_CURRENT_FOLDER_TREE);

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
    let api_key = cli
        .api_key
        .map(ApiKeySource::Text)
        .or(cli.api_key_command.map(ApiKeySource::Command))
        .or(cli.api_key_env.map(ApiKeySource::Env))
        .or(env_cfg.api_key)
        .or(file_cfg.api_key);

    let embedding_provider = cli
        .embedding_provider
        .or(env_cfg.embedding_provider)
        .or(file_cfg.embedding_provider)
        .unwrap_or(llm_provider);
    let embedding_model = cli
        .embedding_model
        .or(env_cfg.embedding_model)
        .or(file_cfg.embedding_model)
        .unwrap_or_else(|| default_embedding_model(embedding_provider, &llm_model));
    let embedding_base_url = cli
        .embedding_base_url
        .or(env_cfg.embedding_base_url)
        .or(file_cfg.embedding_base_url)
        .or_else(|| llm_base_url.clone());
    let embedding_api_key = cli
        .embedding_api_key
        .map(ApiKeySource::Text)
        .or(cli.embedding_api_key_command.map(ApiKeySource::Command))
        .or(cli.embedding_api_key_env.map(ApiKeySource::Env))
        .or(env_cfg.embedding_api_key)
        .or(file_cfg.embedding_api_key)
        .or_else(|| api_key.clone());
    let keyword_batch_size = cli
        .keyword_batch_size
        .or(env_cfg.keyword_batch_size)
        .or(file_cfg.keyword_batch_size)
        .unwrap_or(DEFAULT_KEYWORD_BATCH_SIZE);
    let batch_start_delay_ms = env_cfg
        .batch_start_delay_ms
        .or(file_cfg.batch_start_delay_ms)
        .unwrap_or(DEFAULT_BATCH_START_DELAY_MS);
    let subcategories_suggestion_number = cli
        .subcategories_suggestion_number
        .or(env_cfg.subcategories_suggestion_number)
        .or(file_cfg.subcategories_suggestion_number)
        .unwrap_or(DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER);

    validate_non_zero("max_file_size_mb", &max_file_size_mb)?;
    validate_non_zero("page_cutoff", &page_cutoff)?;
    validate_non_zero("pdf_extract_workers", &pdf_extract_workers)?;
    validate_non_zero("category_depth", &category_depth)?;
    validate_non_zero("taxonomy_batch_size", &taxonomy_batch_size)?;
    validate_non_zero("reference_top_k", &reference_top_k)?;
    validate_non_zero("placement_batch_size", &placement_batch_size)?;
    validate_non_zero("keyword_batch_size", &keyword_batch_size)?;
    validate_non_zero(
        "subcategories_suggestion_number",
        &subcategories_suggestion_number,
    )?;

    Ok(AppConfig {
        input,
        output,
        recursive,
        max_file_size_mb,
        page_cutoff,
        pdf_extract_workers,
        category_depth,
        taxonomy_mode,
        taxonomy_assistance,
        taxonomy_batch_size,
        reference_manifest_path,
        reference_top_k,
        use_current_folder_tree,
        placement_batch_size,
        placement_mode,
        rebuild,
        dry_run,
        llm_provider,
        llm_model,
        llm_base_url,
        api_key,
        embedding_provider,
        embedding_model,
        embedding_base_url,
        embedding_api_key,
        keyword_batch_size,
        batch_start_delay_ms,
        subcategories_suggestion_number,
        verbose: verbosity >= 1,
        debug: verbosity >= 2,
        quiet: cli.quiet,
    })
}

fn validate_non_zero<T>(name: &str, value: &T) -> Result<()>
where
    T: Eq + From<u8>,
{
    if *value == T::from(0) {
        return Err(AppError::Validation(format!(
            "{name} must be greater than 0"
        )));
    }
    Ok(())
}

fn normalize_verbosity(raw: u8) -> u8 {
    raw.min(2)
}

fn default_embedding_model(provider: crate::llm::LlmProvider, llm_model: &str) -> String {
    match provider {
        crate::llm::LlmProvider::Openai => DEFAULT_OPENAI_EMBEDDING_MODEL.to_string(),
        crate::llm::LlmProvider::Gemini => DEFAULT_GEMINI_EMBEDDING_MODEL.to_string(),
        crate::llm::LlmProvider::Ollama => llm_model.to_string(),
    }
}
