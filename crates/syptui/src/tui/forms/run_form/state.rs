use crate::{
    cli::{
        DEFAULT_CATEGORY_DEPTH, DEFAULT_INPUT, DEFAULT_KEYWORD_BATCH_SIZE, DEFAULT_LLM_MODEL,
        DEFAULT_LLM_PROVIDER, DEFAULT_MAX_FILE_SIZE_MB, DEFAULT_OUTPUT, DEFAULT_PAGE_CUTOFF,
        DEFAULT_PDF_EXTRACT_WORKERS, DEFAULT_PLACEMENT_BATCH_SIZE,
        DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER, DEFAULT_TAXONOMY_BATCH_SIZE,
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
    pub(crate) field: Option<usize>,
    pub(crate) severity: ValidationSeverity,
    pub(crate) message: String,
}

pub(crate) struct RunFormAnalysis {
    pub(crate) config: Option<AppConfig>,
    pub(crate) issues: Vec<ValidationIssue>,
}

pub(crate) struct RunForm {
    pub(crate) selected: usize,
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
            selected: 0,
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
            llm_model: DEFAULT_LLM_MODEL.to_string(),
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
    pub(crate) const RUN_BUTTON_INDEX: usize = 23;

    pub(crate) const COLUMN_FIELDS: [&'static [usize]; 3] = [
        &[0, 1, 2, 3, 4, 5],
        &[6, 7, 8, 22, 18, 19, 9, 10],
        &[13, 14, 15, 16, 17, 11, 12, 20, 21, Self::RUN_BUTTON_INDEX],
    ];

    pub(crate) const VISIBLE_FIELDS: [usize; 24] = [
        0,
        1,
        2,
        3,
        4,
        5,
        6,
        7,
        8,
        22,
        18,
        19,
        9,
        10,
        13,
        14,
        15,
        16,
        17,
        11,
        12,
        20,
        21,
        Self::RUN_BUTTON_INDEX,
    ];

    pub(crate) fn from_config(config: &AppConfig) -> Self {
        Self {
            selected: 0,
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

    pub(crate) fn select_next(&mut self) {
        if let Some(index) = Self::VISIBLE_FIELDS
            .iter()
            .position(|field| *field == self.selected)
        {
            self.selected = Self::VISIBLE_FIELDS[(index + 1).min(Self::VISIBLE_FIELDS.len() - 1)];
        } else {
            self.selected = Self::VISIBLE_FIELDS[0];
        }
    }

    pub(crate) fn select_previous(&mut self) {
        if let Some(index) = Self::VISIBLE_FIELDS
            .iter()
            .position(|field| *field == self.selected)
        {
            self.selected = Self::VISIBLE_FIELDS[index.saturating_sub(1)];
        } else {
            self.selected = Self::VISIBLE_FIELDS[0];
        }
    }

    pub(crate) fn move_column_left(&mut self) {
        self.move_column(-1);
    }

    pub(crate) fn move_column_right(&mut self) {
        self.move_column(1);
    }

    pub(crate) fn editable(&self, index: usize) -> bool {
        !matches!(
            index,
            2 | 7 | 10 | 11 | 12 | 13 | 16 | 20 | 21 | 22 | Self::RUN_BUTTON_INDEX
        )
    }

    pub(crate) fn toggle_selected(&mut self) {
        match self.selected {
            2 => self.recursive = !self.recursive,
            11 => self.rebuild = !self.rebuild,
            12 => self.apply = !self.apply,
            21 => self.quiet = !self.quiet,
            22 => self.use_current_folder_tree = !self.use_current_folder_tree,
            _ => self.cycle_selected(1),
        }
    }

    pub(crate) fn cycle_selected(&mut self, direction: i8) {
        match self.selected {
            7 => self.taxonomy_mode = cycle_taxonomy_mode(self.taxonomy_mode, direction),
            10 => self.placement_mode = cycle_placement_mode(self.placement_mode, direction),
            13 => self.llm_provider = cycle_provider(self.llm_provider, direction),
            16 => {
                self.api_key_source = if direction >= 0 {
                    self.api_key_source.next()
                } else {
                    self.api_key_source.previous()
                }
            }
            20 => {
                self.verbosity = if direction >= 0 {
                    self.verbosity.next()
                } else {
                    self.verbosity.previous()
                }
            }
            _ => {}
        }
    }

    pub(crate) fn apply_edit(&mut self, value: String) -> Result<()> {
        match self.selected {
            0 => self.input = value,
            1 => self.output = value,
            3 => self.max_file_size_mb = value,
            4 => self.page_cutoff = value,
            5 => self.pdf_extract_workers = value,
            6 => self.category_depth = value,
            8 => self.taxonomy_batch_size = value,
            9 => self.placement_batch_size = value,
            14 => self.llm_model = value,
            15 => self.llm_base_url = value,
            17 => self.api_key_value = value,
            18 => self.keyword_batch_size = value,
            19 => self.subcategories_suggestion_number = value,
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn value(&self, index: usize) -> String {
        match index {
            0 => self.input.clone(),
            1 => self.output.clone(),
            2 => bool_label(self.recursive).to_string(),
            3 => self.max_file_size_mb.clone(),
            4 => self.page_cutoff.clone(),
            5 => self.pdf_extract_workers.clone(),
            6 => self.category_depth.clone(),
            7 => taxonomy_mode_label(self.taxonomy_mode).to_string(),
            8 => self.taxonomy_batch_size.clone(),
            9 => self.placement_batch_size.clone(),
            10 => placement_mode_label(self.placement_mode).to_string(),
            11 => bool_label(self.rebuild).to_string(),
            12 => bool_label(self.apply).to_string(),
            13 => provider_label(self.llm_provider).to_string(),
            14 => self.llm_model.clone(),
            15 => self.llm_base_url.clone(),
            16 => self.api_key_source.label().to_string(),
            17 => {
                super::validation::api_key_value_display(self.api_key_source, &self.api_key_value)
            }
            18 => self.keyword_batch_size.clone(),
            19 => self.subcategories_suggestion_number.clone(),
            20 => self.verbosity.label().to_string(),
            21 => bool_label(self.quiet).to_string(),
            22 => bool_label(self.use_current_folder_tree).to_string(),
            Self::RUN_BUTTON_INDEX => "Press `Enter`, `Space`, or `r` to launch.".to_string(),
            _ => String::new(),
        }
    }

    pub(crate) fn run_button_selected(&self) -> bool {
        self.selected == Self::RUN_BUTTON_INDEX
    }

    fn move_column(&mut self, direction: i8) {
        let Some((column_index, row_index)) = Self::column_position(self.selected) else {
            self.selected = Self::VISIBLE_FIELDS[0];
            return;
        };

        let target_column = if direction < 0 {
            column_index.saturating_sub(1)
        } else {
            (column_index + 1).min(Self::COLUMN_FIELDS.len() - 1)
        };

        if target_column == column_index {
            return;
        }

        let target_fields = Self::COLUMN_FIELDS[target_column];
        self.selected = target_fields[row_index.min(target_fields.len() - 1)];
    }

    fn column_position(field: usize) -> Option<(usize, usize)> {
        Self::COLUMN_FIELDS
            .iter()
            .enumerate()
            .find_map(|(column_index, fields)| {
                fields
                    .iter()
                    .position(|candidate| *candidate == field)
                    .map(|row_index| (column_index, row_index))
            })
    }
}
