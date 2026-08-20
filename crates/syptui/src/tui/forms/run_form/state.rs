use crate::{
    cli::{
        DEFAULT_CATEGORY_DEPTH, DEFAULT_INPUT, DEFAULT_KEYWORD_BATCH_SIZE, DEFAULT_LLM_PROVIDER,
        DEFAULT_MAX_FILE_SIZE_MB, DEFAULT_OUTPUT, DEFAULT_PAGE_CUTOFF, DEFAULT_PDF_EXTRACT_WORKERS,
        DEFAULT_PLACEMENT_BATCH_SIZE, DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER,
        DEFAULT_TAXONOMY_BATCH_SIZE, default_llm_model,
    },
    config::AppConfig,
    error::Result,
    llm::LlmProvider,
    papers::placement::PlacementMode,
    papers::taxonomy::TaxonomyMode,
};

use crate::tui::forms::{
    ApiKeySourceMode, UiVerbosity, bool_label, cycle_placement_mode, cycle_provider,
    cycle_taxonomy_mode, placement_mode_label, provider_label, taxonomy_mode_label,
};

use super::fields::{COLUMNS, RunField, visible_column_fields, visible_fields};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValidationSeverity {
    Info,
    Warning,
    Error,
}

impl ValidationSeverity {
    pub(crate) fn rank(self) -> u8 {
        match self {
            Self::Info => 0,
            Self::Warning => 1,
            Self::Error => 2,
        }
    }

    pub(crate) fn marker(self) -> char {
        match self {
            Self::Info => 'i',
            Self::Warning => '~',
            Self::Error => '!',
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Warning => "Warning",
            Self::Error => "Error",
        }
    }

    pub(crate) fn color(self) -> ratatui::prelude::Color {
        match self {
            Self::Info => ratatui::prelude::Color::Cyan,
            Self::Warning => ratatui::prelude::Color::Yellow,
            Self::Error => ratatui::prelude::Color::Red,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ValidationIssue {
    pub(crate) field: Option<RunField>,
    pub(crate) severity: ValidationSeverity,
    pub(crate) message: String,
}

pub(crate) struct RunFormAnalysis {
    pub(crate) config: Option<AppConfig>,
    pub(crate) issues: Vec<ValidationIssue>,
}

pub(crate) struct RunForm {
    pub(crate) selected: RunField,
    /// Shows tuning fields that are hidden from the default form.
    pub(crate) advanced: bool,
    pub(crate) input: String,
    pub(crate) output: String,
    pub(crate) recursive: bool,
    pub(crate) max_file_size_mb: String,
    pub(crate) page_cutoff: String,
    pub(crate) pdf_extract_workers: String,
    pub(crate) category_depth: String,
    pub(crate) taxonomy_mode: TaxonomyMode,
    pub(crate) taxonomy_batch_size: String,
    pub(crate) use_current_folder_tree: bool,
    pub(crate) placement_batch_size: String,
    pub(crate) placement_mode: PlacementMode,
    pub(crate) rebuild: bool,
    pub(crate) apply: bool,
    pub(crate) llm_provider: LlmProvider,
    pub(crate) llm_model: String,
    pub(crate) llm_base_url: String,
    pub(crate) api_key_source: ApiKeySourceMode,
    pub(crate) api_key_value: String,
    pub(crate) keyword_batch_size: String,
    pub(crate) subcategories_suggestion_number: String,
    pub(crate) verbosity: UiVerbosity,
    pub(crate) quiet: bool,
}

impl Default for RunForm {
    fn default() -> Self {
        Self {
            selected: RunField::Input,
            advanced: false,
            input: DEFAULT_INPUT.to_string(),
            output: DEFAULT_OUTPUT.to_string(),
            recursive: false,
            max_file_size_mb: DEFAULT_MAX_FILE_SIZE_MB.to_string(),
            page_cutoff: DEFAULT_PAGE_CUTOFF.to_string(),
            pdf_extract_workers: DEFAULT_PDF_EXTRACT_WORKERS.to_string(),
            category_depth: DEFAULT_CATEGORY_DEPTH.to_string(),
            taxonomy_mode: TaxonomyMode::BatchMerge,
            taxonomy_batch_size: DEFAULT_TAXONOMY_BATCH_SIZE.to_string(),
            use_current_folder_tree: false,
            placement_batch_size: DEFAULT_PLACEMENT_BATCH_SIZE.to_string(),
            placement_mode: PlacementMode::ExistingOnly,
            rebuild: false,
            apply: false,
            llm_provider: DEFAULT_LLM_PROVIDER,
            llm_model: default_llm_model(DEFAULT_LLM_PROVIDER).to_string(),
            llm_base_url: String::new(),
            api_key_source: ApiKeySourceMode::Text,
            api_key_value: String::new(),
            keyword_batch_size: DEFAULT_KEYWORD_BATCH_SIZE.to_string(),
            subcategories_suggestion_number: DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER.to_string(),
            verbosity: UiVerbosity::Normal,
            quiet: false,
        }
    }
}

impl RunForm {
    pub(crate) fn from_config(config: &AppConfig) -> Self {
        Self {
            selected: RunField::Input,
            advanced: false,
            input: config.input.display().to_string(),
            output: config.output.display().to_string(),
            recursive: config.recursive,
            max_file_size_mb: config.max_file_size_mb.to_string(),
            page_cutoff: config.page_cutoff.to_string(),
            pdf_extract_workers: config.pdf_extract_workers.to_string(),
            category_depth: config.category_depth.to_string(),
            taxonomy_mode: config.taxonomy_mode,
            taxonomy_batch_size: config.taxonomy_batch_size.to_string(),
            use_current_folder_tree: config.use_current_folder_tree,
            placement_batch_size: config.placement_batch_size.to_string(),
            placement_mode: config.placement_mode,
            rebuild: config.rebuild,
            apply: !config.dry_run,
            llm_provider: config.llm_provider,
            llm_model: config.llm_model.clone(),
            llm_base_url: config.llm_base_url.clone().unwrap_or_default(),
            api_key_source: match &config.api_key {
                Some(crate::config::ApiKeySource::Text(_)) | None => ApiKeySourceMode::Text,
                Some(crate::config::ApiKeySource::Command(_)) => ApiKeySourceMode::Command,
                Some(crate::config::ApiKeySource::Env(_)) => ApiKeySourceMode::Env,
            },
            api_key_value: match &config.api_key {
                Some(crate::config::ApiKeySource::Text(value))
                | Some(crate::config::ApiKeySource::Command(value))
                | Some(crate::config::ApiKeySource::Env(value)) => value.clone(),
                None => String::new(),
            },
            keyword_batch_size: config.keyword_batch_size.to_string(),
            subcategories_suggestion_number: config.subcategories_suggestion_number.to_string(),
            verbosity: if config.debug {
                UiVerbosity::Debug
            } else if config.verbose {
                UiVerbosity::Verbose
            } else {
                UiVerbosity::Normal
            },
            quiet: config.quiet,
        }
    }

    /// Shows or hides the advanced fields, keeping the selection on a field
    /// that is still visible.
    pub(crate) fn toggle_advanced(&mut self) {
        self.advanced = !self.advanced;
        if !self.selected.visible(self.advanced) {
            self.selected = RunField::Input;
        }
    }

    pub(crate) fn select_next(&mut self) {
        self.select_by_offset(1);
    }

    pub(crate) fn select_previous(&mut self) {
        self.select_by_offset(-1);
    }

    pub(crate) fn move_column_left(&mut self) {
        self.move_column(-1);
    }

    pub(crate) fn move_column_right(&mut self) {
        self.move_column(1);
    }

    pub(crate) fn toggle_selected(&mut self) {
        match self.selected {
            RunField::Recursive => self.recursive = !self.recursive,
            RunField::Rebuild => self.rebuild = !self.rebuild,
            RunField::Apply => self.apply = !self.apply,
            RunField::Quiet => self.quiet = !self.quiet,
            RunField::UseCurrentFolderTree => {
                self.use_current_folder_tree = !self.use_current_folder_tree;
            }
            _ => self.cycle_selected(1),
        }
    }

    pub(crate) fn cycle_selected(&mut self, direction: i8) {
        match self.selected {
            RunField::TaxonomyMode => {
                self.taxonomy_mode = cycle_taxonomy_mode(self.taxonomy_mode, direction);
            }
            RunField::PlacementMode => {
                self.placement_mode = cycle_placement_mode(self.placement_mode, direction);
            }
            RunField::LlmProvider => {
                let previous = self.llm_provider;
                self.llm_provider = cycle_provider(previous, direction);
                // Model names are provider-specific. Carry the new provider's
                // default across, unless the field holds something typed.
                if self.llm_model == default_llm_model(previous) {
                    self.llm_model = default_llm_model(self.llm_provider).to_string();
                }
            }
            RunField::ApiKeySource => {
                self.api_key_source = if direction >= 0 {
                    self.api_key_source.next()
                } else {
                    self.api_key_source.previous()
                };
            }
            RunField::Verbosity => {
                self.verbosity = if direction >= 0 {
                    self.verbosity.next()
                } else {
                    self.verbosity.previous()
                };
            }
            _ => {}
        }
    }

    pub(crate) fn apply_edit(&mut self, value: String) -> Result<()> {
        match self.selected {
            RunField::Input => self.input = value,
            RunField::Output => self.output = value,
            RunField::MaxFileSizeMb => self.max_file_size_mb = value,
            RunField::PageCutoff => self.page_cutoff = value,
            RunField::PdfExtractWorkers => self.pdf_extract_workers = value,
            RunField::CategoryDepth => self.category_depth = value,
            RunField::TaxonomyBatchSize => self.taxonomy_batch_size = value,
            RunField::PlacementBatchSize => self.placement_batch_size = value,
            RunField::LlmModel => self.llm_model = value,
            RunField::LlmBaseUrl => self.llm_base_url = value,
            RunField::ApiKeyValue => self.api_key_value = value,
            RunField::KeywordBatchSize => self.keyword_batch_size = value,
            RunField::SubcategoriesSuggestionNumber => {
                self.subcategories_suggestion_number = value;
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn value(&self, field: RunField) -> String {
        match field {
            RunField::Input => self.input.clone(),
            RunField::Output => self.output.clone(),
            RunField::Recursive => bool_label(self.recursive).to_string(),
            RunField::MaxFileSizeMb => self.max_file_size_mb.clone(),
            RunField::PageCutoff => self.page_cutoff.clone(),
            RunField::PdfExtractWorkers => self.pdf_extract_workers.clone(),
            RunField::CategoryDepth => self.category_depth.clone(),
            RunField::TaxonomyMode => taxonomy_mode_label(self.taxonomy_mode).to_string(),
            RunField::TaxonomyBatchSize => self.taxonomy_batch_size.clone(),
            RunField::UseCurrentFolderTree => bool_label(self.use_current_folder_tree).to_string(),
            RunField::KeywordBatchSize => self.keyword_batch_size.clone(),
            RunField::SubcategoriesSuggestionNumber => self.subcategories_suggestion_number.clone(),
            RunField::PlacementMode => placement_mode_label(self.placement_mode).to_string(),
            RunField::PlacementBatchSize => self.placement_batch_size.clone(),
            RunField::LlmProvider => provider_label(self.llm_provider).to_string(),
            RunField::LlmModel => self.llm_model.clone(),
            RunField::LlmBaseUrl => self.llm_base_url.clone(),
            RunField::ApiKeySource => self.api_key_source.label().to_string(),
            RunField::ApiKeyValue => {
                super::validation::api_key_value_display(self.api_key_source, &self.api_key_value)
            }
            RunField::Apply => bool_label(self.apply).to_string(),
            RunField::Rebuild => bool_label(self.rebuild).to_string(),
            RunField::Verbosity => self.verbosity.label().to_string(),
            RunField::Quiet => bool_label(self.quiet).to_string(),
            RunField::RunButton => "Press `Enter`, `Space`, or `r` to launch.".to_string(),
        }
    }

    pub(crate) fn run_button_selected(&self) -> bool {
        self.selected == RunField::RunButton
    }

    fn select_by_offset(&mut self, direction: i8) {
        let fields = visible_fields(self.advanced);
        let Some(index) = fields.iter().position(|field| *field == self.selected) else {
            self.selected = fields[0];
            return;
        };

        self.selected = if direction < 0 {
            fields[index.saturating_sub(1)]
        } else {
            fields[(index + 1).min(fields.len() - 1)]
        };
    }

    fn move_column(&mut self, direction: i8) {
        let Some((column_index, row_index)) = self.column_position(self.selected) else {
            self.selected = visible_fields(self.advanced)[0];
            return;
        };

        let target_column = if direction < 0 {
            column_index.saturating_sub(1)
        } else {
            (column_index + 1).min(COLUMNS.len() - 1)
        };

        if target_column == column_index {
            return;
        }

        let target_fields = visible_column_fields(target_column, self.advanced);
        if target_fields.is_empty() {
            return;
        }
        self.selected = target_fields[row_index.min(target_fields.len() - 1)];
    }

    fn column_position(&self, field: RunField) -> Option<(usize, usize)> {
        (0..COLUMNS.len()).find_map(|column_index| {
            visible_column_fields(column_index, self.advanced)
                .iter()
                .position(|candidate| *candidate == field)
                .map(|row_index| (column_index, row_index))
        })
    }
}
