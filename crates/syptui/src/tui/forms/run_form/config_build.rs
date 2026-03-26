use std::path::PathBuf;

use crate::tui::forms::{
    ApiKeySourceMode, empty_string_to_option, parse_u8, parse_u64, parse_usize,
};
use crate::{CliArgs, config, config::AppConfig, error::Result};

use super::RunForm;

impl RunForm {
    /// Builds a resolved app config from the current form values.
    ///
    /// # Errors
    /// Returns an error when the form cannot be converted into a valid config.
    pub(crate) fn build_config(&self) -> Result<AppConfig> {
        let cli = CliArgs {
            input: Some(PathBuf::from(self.input.trim())),
            output: Some(PathBuf::from(self.output.trim())),
            recursive: Some(self.recursive),
            max_file_size_mb: Some(parse_u64("max_file_size_mb", &self.max_file_size_mb)?),
            page_cutoff: Some(parse_u8("page_cutoff", &self.page_cutoff)?),
            pdf_extract_workers: Some(parse_usize(
                "pdf_extract_workers",
                &self.pdf_extract_workers,
            )?),
            category_depth: Some(parse_u8("category_depth", &self.category_depth)?),
            taxonomy_mode: Some(self.taxonomy_mode),
            taxonomy_assistance: None,
            taxonomy_batch_size: Some(parse_usize(
                "taxonomy_batch_size",
                &self.taxonomy_batch_size,
            )?),
            reference_manifest_path: None,
            reference_top_k: None,
            use_current_folder_tree: Some(self.use_current_folder_tree),
            placement_batch_size: Some(parse_usize(
                "placement_batch_size",
                &self.placement_batch_size,
            )?),
            placement_assistance: None,
            placement_mode: Some(self.placement_mode),
            placement_reference_top_k: None,
            placement_candidate_top_k: None,
            placement_min_similarity: None,
            placement_min_margin: None,
            placement_min_reference_support: None,
            rebuild: Some(self.rebuild),
            apply: self.apply,
            llm_provider: Some(self.llm_provider),
            llm_model: Some(self.llm_model.trim().to_string()),
            llm_base_url: empty_string_to_option(&self.llm_base_url),
            api_key: (self.api_key_source == ApiKeySourceMode::Text)
                .then(|| empty_string_to_option(&self.api_key_value))
                .flatten(),
            api_key_command: (self.api_key_source == ApiKeySourceMode::Command)
                .then(|| empty_string_to_option(&self.api_key_value))
                .flatten(),
            api_key_env: (self.api_key_source == ApiKeySourceMode::Env)
                .then(|| empty_string_to_option(&self.api_key_value))
                .flatten(),
            embedding_provider: None,
            embedding_model: None,
            embedding_base_url: None,
            embedding_api_key: None,
            embedding_api_key_command: None,
            embedding_api_key_env: None,
            keyword_batch_size: Some(parse_usize("keyword_batch_size", &self.keyword_batch_size)?),
            subcategories_suggestion_number: Some(parse_usize(
                "subcategories_suggestion_number",
                &self.subcategories_suggestion_number,
            )?),
            verbosity: self.verbosity.raw(),
            quiet: self.quiet,
        };
        config::resolve_config(cli)
    }
}
