use std::{
    env, fs,
    path::{Path, PathBuf},
};

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Modifier, Span, Style},
    widgets::{ListItem, Paragraph, Wrap},
};

use crate::{
    CliArgs,
    cli::{
        DEFAULT_CATEGORY_DEPTH, DEFAULT_INPUT, DEFAULT_KEYWORD_BATCH_SIZE, DEFAULT_LLM_MODEL,
        DEFAULT_LLM_PROVIDER, DEFAULT_MAX_FILE_SIZE_MB, DEFAULT_OUTPUT, DEFAULT_PAGE_CUTOFF,
        DEFAULT_PDF_EXTRACT_WORKERS, DEFAULT_PLACEMENT_BATCH_SIZE,
        DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER, DEFAULT_TAXONOMY_BATCH_SIZE,
    },
    config,
    config::AppConfig,
    error::Result,
    llm::LlmProvider,
    papers::placement::PlacementMode,
    papers::taxonomy::TaxonomyMode,
};

use super::{
    ApiKeySourceMode, UiVerbosity, bool_label, cycle_placement_mode, cycle_provider,
    cycle_taxonomy_mode, empty_string_to_option, masked_value, parse_u8, parse_u64, parse_usize,
    placement_mode_label, provider_label, run_field_help, run_field_key, run_field_label,
    taxonomy_mode_label,
};
use crate::tui::{
    theme::ThemePalette,
    ui_widgets::{render_selectable_list, stylized_body_line},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValidationSeverity {
    Info,
    Warning,
    Error,
}

impl ValidationSeverity {
    fn rank(self) -> u8 {
        match self {
            Self::Info => 0,
            Self::Warning => 1,
            Self::Error => 2,
        }
    }

    fn marker(self) -> char {
        match self {
            Self::Info => 'i',
            Self::Warning => '~',
            Self::Error => '!',
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Warning => "Warning",
            Self::Error => "Error",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Info => Color::Cyan,
            Self::Warning => Color::Yellow,
            Self::Error => Color::Red,
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
    issues: Vec<ValidationIssue>,
}

impl RunFormAnalysis {
    pub(crate) fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == ValidationSeverity::Error)
    }

    pub(crate) fn field_issue(&self, field: usize) -> Option<&ValidationIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.field == Some(field))
            .max_by_key(|issue| issue.severity.rank())
    }

    pub(crate) fn blocking_message(&self) -> String {
        let errors = self
            .issues
            .iter()
            .filter(|issue| issue.severity == ValidationSeverity::Error)
            .collect::<Vec<_>>();

        if errors.is_empty() {
            return "No blocking validation issues.".to_string();
        }

        let mut lines = vec!["Fix these issues before starting a run:".to_string()];
        for issue in errors.into_iter().take(4) {
            if let Some(field) = issue.field {
                lines.push(format!("- {}: {}", run_field_label(field), issue.message));
            } else {
                lines.push(format!("- {}", issue.message));
            }
        }
        lines.join("\n")
    }

    fn readiness_text(&self) -> String {
        let warnings = self
            .issues
            .iter()
            .filter(|issue| issue.severity == ValidationSeverity::Warning)
            .count();
        let infos = self
            .issues
            .iter()
            .filter(|issue| issue.severity == ValidationSeverity::Info)
            .count();

        if self.has_errors() {
            format!(
                "Fix {} blocking issue(s) before launch",
                self.issues
                    .iter()
                    .filter(|issue| issue.severity == ValidationSeverity::Error)
                    .count()
            )
        } else if warnings > 0 {
            format!("Ready to run with {warnings} warning(s) and {infos} note(s)")
        } else if infos > 0 {
            format!("Ready to run with {infos} note(s)")
        } else {
            "Ready to run".to_string()
        }
    }

    fn issue_counts(&self) -> (usize, usize, usize) {
        let mut infos = 0;
        let mut warnings = 0;
        let mut errors = 0;

        for issue in &self.issues {
            match issue.severity {
                ValidationSeverity::Info => infos += 1,
                ValidationSeverity::Warning => warnings += 1,
                ValidationSeverity::Error => errors += 1,
            }
        }

        (infos, warnings, errors)
    }
}

pub(crate) struct RunForm {
    pub(crate) selected: usize,
    pub(crate) input: String,
    pub(crate) output: String,
    pub(crate) recursive: bool,
    max_file_size_mb: String,
    page_cutoff: String,
    pdf_extract_workers: String,
    category_depth: String,
    pub(crate) taxonomy_mode: TaxonomyMode,
    taxonomy_batch_size: String,
    pub(crate) use_current_folder_tree: bool,
    placement_batch_size: String,
    pub(crate) placement_mode: PlacementMode,
    pub(crate) rebuild: bool,
    pub(crate) apply: bool,
    llm_provider: LlmProvider,
    llm_model: String,
    llm_base_url: String,
    api_key_source: ApiKeySourceMode,
    api_key_value: String,
    keyword_batch_size: String,
    subcategories_suggestion_number: String,
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

pub(in crate::tui) fn list_relative_directories(cwd: &Path, value: &str) -> Vec<String> {
    let query = DirectoryQuery::from_input(cwd, value);
    let Ok(entries) = fs::read_dir(&query.search_dir) else {
        return Vec::new();
    };

    let mut directories = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_dir().then_some(entry.file_name())
        })
        .filter_map(|name| {
            let name = name.to_str()?.to_string();
            if !query.prefix.is_empty() && !name.starts_with(&query.prefix) {
                return None;
            }

            let suggestion = if query.display_dir.as_os_str().is_empty() {
                PathBuf::from(&name)
            } else {
                query.display_dir.join(&name)
            };
            Some(suggestion.display().to_string())
        })
        .collect::<Vec<_>>();

    directories.sort_by_cached_key(|path| path.to_ascii_lowercase());
    directories
}

struct DirectoryQuery {
    search_dir: PathBuf,
    display_dir: PathBuf,
    prefix: String,
}

impl DirectoryQuery {
    fn from_input(cwd: &Path, value: &str) -> Self {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Self {
                search_dir: cwd.to_path_buf(),
                display_dir: PathBuf::new(),
                prefix: String::new(),
            };
        }

        let typed = PathBuf::from(trimmed);
        let resolved = if typed.is_absolute() {
            typed.clone()
        } else {
            cwd.join(&typed)
        };
        let ends_with_separator = trimmed.chars().last().is_some_and(std::path::is_separator);
        if ends_with_separator || resolved.is_dir() {
            return Self {
                search_dir: resolved,
                display_dir: typed,
                prefix: String::new(),
            };
        }

        Self {
            search_dir: resolved
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| cwd.to_path_buf()),
            display_dir: typed.parent().map(Path::to_path_buf).unwrap_or_default(),
            prefix: typed
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string(),
        }
    }
}

impl RunForm {
    pub(crate) const RUN_BUTTON_INDEX: usize = 23;

    const COLUMN_FIELDS: [&'static [usize]; 3] = [
        &[0, 1, 2, 3, 4, 5],
        &[6, 7, 8, 22, 18, 19, 9, 10],
        &[13, 14, 15, 16, 17, 11, 12, 20, 21, Self::RUN_BUTTON_INDEX],
    ];

    const VISIBLE_FIELDS: [usize; 24] = [
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

    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect, theme: ThemePalette) {
        let analysis = self.analysis();
        let (chunks, side_chunks) = if area.width < 140 {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
                .split(area);
            let side_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
                .split(chunks[1]);
            (chunks, side_chunks)
        } else {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(64), Constraint::Percentage(36)])
                .split(area);
            let side_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
                .split(chunks[1]);
            (chunks, side_chunks)
        };

        self.draw_form_workspace(frame, chunks[0], &analysis, theme);
        self.draw_summary(frame, side_chunks[0], &analysis, theme);
        self.draw_selected_field(frame, side_chunks[1], &analysis, theme);
    }

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

    pub(crate) fn analysis(&self) -> RunFormAnalysis {
        let mut issues = Vec::new();

        self.validate_required_directory(0, &self.input, true, &mut issues);
        self.validate_required_directory(1, &self.output, false, &mut issues);

        if self.llm_model.trim().is_empty() {
            issues.push(ValidationIssue {
                field: Some(14),
                severity: ValidationSeverity::Error,
                message: "Model is required.".to_string(),
            });
        }

        self.validate_numeric_field(3, &self.max_file_size_mb, parse_u64, &mut issues);
        self.validate_numeric_field(4, &self.page_cutoff, parse_u8, &mut issues);
        self.validate_numeric_field(5, &self.pdf_extract_workers, parse_usize, &mut issues);
        self.validate_numeric_field(6, &self.category_depth, parse_u8, &mut issues);
        self.validate_numeric_field(8, &self.taxonomy_batch_size, parse_usize, &mut issues);
        self.validate_numeric_field(9, &self.placement_batch_size, parse_usize, &mut issues);
        self.validate_numeric_field(18, &self.keyword_batch_size, parse_usize, &mut issues);
        self.validate_numeric_field(
            19,
            &self.subcategories_suggestion_number,
            parse_usize,
            &mut issues,
        );

        if matches!(self.llm_provider, LlmProvider::Openai | LlmProvider::Gemini)
            && self.api_key_value.trim().is_empty()
        {
            issues.push(ValidationIssue {
                field: Some(17),
                severity: ValidationSeverity::Warning,
                message: format!(
                    "{} usually requires an API key unless credentials are supplied elsewhere.",
                    provider_label(self.llm_provider)
                ),
            });
        }

        if matches!(self.llm_provider, LlmProvider::Ollama) && !self.api_key_value.trim().is_empty()
        {
            issues.push(ValidationIssue {
                field: Some(17),
                severity: ValidationSeverity::Info,
                message: "Ollama does not use the API key fields.".to_string(),
            });
        }

        if self.api_key_value.trim().is_empty()
            && matches!(
                self.api_key_source,
                ApiKeySourceMode::Command | ApiKeySourceMode::Env
            )
        {
            issues.push(ValidationIssue {
                field: Some(17),
                severity: ValidationSeverity::Warning,
                message: match self.api_key_source {
                    ApiKeySourceMode::Command => {
                        "Enter a shell command that prints the API key.".to_string()
                    }
                    ApiKeySourceMode::Env => {
                        "Enter the environment variable name that holds the API key.".to_string()
                    }
                    ApiKeySourceMode::Text => String::new(),
                },
            });
        }

        if self.api_key_source == ApiKeySourceMode::Env && !self.api_key_value.trim().is_empty() {
            match env::var(self.api_key_value.trim()) {
                Ok(value) if !value.trim().is_empty() => {}
                Ok(_) => issues.push(ValidationIssue {
                    field: Some(17),
                    severity: ValidationSeverity::Warning,
                    message: format!(
                        "Environment variable {} is set but empty.",
                        self.api_key_value.trim()
                    ),
                }),
                Err(_) => issues.push(ValidationIssue {
                    field: Some(17),
                    severity: ValidationSeverity::Warning,
                    message: format!(
                        "Environment variable {} is not set.",
                        self.api_key_value.trim()
                    ),
                }),
            }
        }

        if self.quiet && !matches!(self.verbosity, UiVerbosity::Normal) {
            issues.push(ValidationIssue {
                field: Some(21),
                severity: ValidationSeverity::Warning,
                message: "Quiet mode will suppress most of the extra output from verbose/debug."
                    .to_string(),
            });
        }

        if self.apply && self.rebuild {
            issues.push(ValidationIssue {
                field: Some(11),
                severity: ValidationSeverity::Warning,
                message: "Rebuild + apply will reclassify against a rebuilt taxonomy and then move files."
                    .to_string(),
            });
        }

        if self.use_current_folder_tree && self.rebuild {
            issues.push(ValidationIssue {
                field: Some(22),
                severity: ValidationSeverity::Info,
                message: "Rebuild ignores the current output tree, so this taxonomy hint will be inactive."
                    .to_string(),
            });
        }

        let config = if issues
            .iter()
            .any(|issue| issue.severity == ValidationSeverity::Error)
        {
            None
        } else {
            match self.build_config() {
                Ok(config) => Some(config),
                Err(err) => {
                    issues.push(ValidationIssue {
                        field: None,
                        severity: ValidationSeverity::Error,
                        message: err.to_string(),
                    });
                    None
                }
            }
        };

        RunFormAnalysis { config, issues }
    }

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
            placement_mode: Some(self.placement_mode),
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
            17 => api_key_value_display(self.api_key_source, &self.api_key_value),
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

    fn draw_form_workspace(
        &self,
        frame: &mut Frame,
        area: Rect,
        analysis: &RunFormAnalysis,
        theme: ThemePalette,
    ) {
        let outer = theme.block("Run Setup");
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let chunks = if inner.width < 120 {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(34),
                    Constraint::Percentage(33),
                    Constraint::Percentage(33),
                ])
                .split(inner)
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(34),
                    Constraint::Percentage(33),
                    Constraint::Percentage(33),
                ])
                .split(inner)
        };

        const COLUMN_SECTIONS: [[(&str, &[usize]); 2]; 3] = [
            [("Paths & Scope", &[0, 1, 2]), ("Extraction", &[3, 4, 5])],
            [
                ("Taxonomy", &[6, 7, 8, 22, 18, 19]),
                ("Placement", &[9, 10]),
            ],
            [
                ("LLM & API", &[13, 14, 15, 16, 17]),
                ("Run", &[11, 12, 20, 21, RunForm::RUN_BUTTON_INDEX]),
            ],
        ];

        for (column, sections) in chunks.iter().zip(COLUMN_SECTIONS.iter()) {
            self.draw_column(frame, *column, sections, analysis, theme);
        }
    }

    fn draw_summary(
        &self,
        frame: &mut Frame,
        area: Rect,
        analysis: &RunFormAnalysis,
        theme: ThemePalette,
    ) {
        let (infos, warnings, errors) = analysis.issue_counts();
        let readiness_color = if errors > 0 {
            theme.error
        } else if warnings > 0 {
            theme.warning
        } else {
            theme.success
        };
        let mode_color = if self.apply { theme.error } else { theme.info };
        let output_color = if self.quiet {
            theme.warning
        } else {
            theme.success
        };
        let body_color = theme.panel_fg;
        let mut lines = vec![
            Line::from(vec![
                badge_span("STATUS", readiness_color),
                Span::raw(" "),
                Span::styled(
                    analysis.readiness_text(),
                    Style::default()
                        .fg(readiness_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                badge_span(if self.apply { "APPLY" } else { "PREVIEW" }, mode_color),
                Span::raw(" "),
                badge_span(&format!("{errors} ERR"), theme.error),
                Span::raw(" "),
                badge_span(&format!("{warnings} WARN"), theme.warning),
                Span::raw(" "),
                badge_span(&format!("{infos} NOTE"), theme.info),
            ]),
            Line::from(""),
            section_header_line("Paths", theme.info),
            labeled_value_line(
                "In ",
                &display_path_line(&self.input, DEFAULT_INPUT),
                theme.info,
                body_color,
            ),
            labeled_value_line(
                "Out",
                &display_path_line(&self.output, DEFAULT_OUTPUT),
                theme.info,
                body_color,
            ),
            Line::from(""),
            section_header_line("Pipeline", theme.accent),
            labeled_value_line(
                "Extract",
                &format!(
                    "{} MB | {} page(s) | {} worker(s)",
                    self.max_file_size_mb.trim(),
                    self.page_cutoff.trim(),
                    self.pdf_extract_workers.trim()
                ),
                theme.accent,
                body_color,
            ),
            labeled_value_line(
                "Taxonomy",
                &format!(
                    "depth {} | {} | batch {} | tree {}",
                    self.category_depth.trim(),
                    taxonomy_mode_label(self.taxonomy_mode),
                    self.taxonomy_batch_size.trim(),
                    bool_label(self.use_current_folder_tree)
                ),
                theme.accent,
                body_color,
            ),
            labeled_value_line(
                "Ideas",
                &format!(
                    "keywords {} | suggestions {}",
                    self.keyword_batch_size.trim(),
                    self.subcategories_suggestion_number.trim()
                ),
                theme.accent,
                body_color,
            ),
            labeled_value_line(
                "Place",
                &format!(
                    "{} | batch {}",
                    placement_mode_label(self.placement_mode),
                    self.placement_batch_size.trim()
                ),
                theme.accent,
                body_color,
            ),
            Line::from(""),
            section_header_line("Launch", theme.success),
            labeled_value_line(
                "LLM",
                &format!(
                    "{} / {}",
                    provider_label(self.llm_provider),
                    if self.llm_model.trim().is_empty() {
                        "<missing>"
                    } else {
                        self.llm_model.trim()
                    }
                ),
                theme.success,
                body_color,
            ),
            labeled_value_line(
                "Auth",
                &format!(
                    "{} / {}",
                    self.api_key_source.label(),
                    api_key_summary(self.api_key_source, &self.api_key_value)
                ),
                theme.success,
                body_color,
            ),
            labeled_value_line(
                "Output",
                &format!(
                    "rebuild {} | quiet {} | {}",
                    bool_label(self.rebuild),
                    bool_label(self.quiet),
                    self.verbosity.label()
                ),
                theme.success,
                output_color,
            ),
        ];

        if let Some(config) = &analysis.config {
            lines.push(Line::from(""));
            lines.push(section_header_line("Resolved", theme.warning));
            lines.push(labeled_value_line(
                "Mode",
                if config.dry_run { "preview" } else { "apply" },
                theme.warning,
                body_color,
            ));
            lines.push(labeled_value_line(
                "Scope",
                if self.recursive {
                    "recursive"
                } else {
                    "top-level only"
                },
                theme.warning,
                body_color,
            ));
        }

        let notable_issues = analysis.issues.iter().take(4).collect::<Vec<_>>();
        if !notable_issues.is_empty() {
            lines.push(Line::from(""));
            lines.push(section_header_line("Issues", theme.error));
            for issue in notable_issues {
                lines.push(Line::from(Span::styled(
                    format!(
                        "{} {}",
                        issue.severity.marker(),
                        summarize_issue(issue, analysis)
                    ),
                    Style::default().fg(issue.severity.color()),
                )));
            }
        }

        frame.render_widget(
            Paragraph::new(lines)
                .style(theme.panel_style())
                .wrap(Wrap { trim: false })
                .block(
                    theme
                        .block("Launch Preview")
                        .border_style(Style::default().fg(readiness_color).bg(theme.panel_bg)),
                ),
            area,
        );
    }

    fn draw_selected_field(
        &self,
        frame: &mut Frame,
        area: Rect,
        analysis: &RunFormAnalysis,
        theme: ThemePalette,
    ) {
        let selected_label = run_field_label(self.selected);
        let selected_value = self.value(self.selected);
        let mut lines = vec![
            Line::from(Span::styled(
                selected_label,
                Style::default()
                    .fg(theme.info)
                    .bg(theme.panel_bg)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            section_header_line("Description", theme.info),
            stylized_body_line(run_field_help(self.selected), theme),
            Line::from(""),
            section_header_line("Current", theme.success),
            stylized_body_line(&selected_value, theme),
        ];

        if let Some(issue) = analysis.field_issue(self.selected) {
            lines.push(Line::from(""));
            lines.push(section_header_line("Issue", issue.severity.color()));
            lines.push(Line::from(Span::styled(
                format!("{}: {}", issue.severity.title(), issue.message),
                Style::default()
                    .fg(issue.severity.color())
                    .add_modifier(Modifier::BOLD),
            )));
        }

        frame.render_widget(
            Paragraph::new(lines)
                .style(theme.panel_style())
                .wrap(Wrap { trim: false })
                .block(theme.block("Selected Field")),
            area,
        );
    }

    fn draw_column(
        &self,
        frame: &mut Frame,
        area: Rect,
        sections: &[(&str, &[usize])],
        analysis: &RunFormAnalysis,
        theme: ThemePalette,
    ) {
        let mut items = Vec::new();
        let mut selected_item = None;
        for (section_index, (title, fields)) in sections.iter().enumerate() {
            if section_index > 0 {
                items.push(ListItem::new(""));
            }
            items.push(ListItem::new(Line::from(Span::styled(
                (*title).to_string(),
                Style::default()
                    .fg(theme.info)
                    .bg(theme.panel_bg)
                    .add_modifier(Modifier::BOLD),
            ))));

            for field_index in *fields {
                if *field_index == Self::RUN_BUTTON_INDEX {
                    if *field_index == self.selected {
                        selected_item = Some(items.len());
                    }
                    items.push(ListItem::new(Line::styled(
                        "  [ Run ]  ",
                        Style::default()
                            .fg(theme.selection_fg)
                            .bg(theme.selection_bg)
                            .add_modifier(Modifier::BOLD),
                    )));
                    continue;
                }

                let marker = analysis
                    .field_issue(*field_index)
                    .map_or(' ', |issue| issue.severity.marker());
                let content = format!(
                    "{} {}: {}",
                    marker,
                    run_field_label(*field_index),
                    self.value(*field_index)
                );

                if *field_index == self.selected {
                    selected_item = Some(items.len());
                }

                if let Some(issue) = analysis.field_issue(*field_index) {
                    items.push(ListItem::new(Line::styled(
                        content,
                        Style::default().fg(issue.severity.color()),
                    )));
                } else {
                    items.push(ListItem::new(content));
                }
            }
        }

        render_selectable_list(frame, area, theme.block(""), items, selected_item, theme);
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

    fn validate_required_directory(
        &self,
        field: usize,
        value: &str,
        must_exist: bool,
        issues: &mut Vec<ValidationIssue>,
    ) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            issues.push(ValidationIssue {
                field: Some(field),
                severity: ValidationSeverity::Error,
                message: "Path is required.".to_string(),
            });
            return;
        }

        let path = Path::new(trimmed);
        match fs::metadata(path) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    issues.push(ValidationIssue {
                        field: Some(field),
                        severity: ValidationSeverity::Error,
                        message: "Path exists but is not a directory.".to_string(),
                    });
                }
            }
            Err(_) if must_exist => {
                issues.push(ValidationIssue {
                    field: Some(field),
                    severity: ValidationSeverity::Error,
                    message: "Folder does not exist.".to_string(),
                });
            }
            Err(_) => {
                issues.push(ValidationIssue {
                    field: Some(field),
                    severity: ValidationSeverity::Info,
                    message: "Folder does not exist yet. It will be created during apply mode."
                        .to_string(),
                });
            }
        }
    }

    fn validate_numeric_field<T>(
        &self,
        field: usize,
        value: &str,
        parse: impl Fn(&str, &str) -> Result<T>,
        issues: &mut Vec<ValidationIssue>,
    ) {
        if let Err(err) = parse(run_field_key(field), value) {
            issues.push(ValidationIssue {
                field: Some(field),
                severity: ValidationSeverity::Error,
                message: err.to_string(),
            });
        }
    }
}

fn summarize_issue(issue: &ValidationIssue, analysis: &RunFormAnalysis) -> String {
    let _ = analysis;
    if let Some(field) = issue.field {
        format!("{}: {}", run_field_label(field), issue.message)
    } else {
        issue.message.clone()
    }
}

fn display_path_line(value: &str, default_value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        default_value.to_string()
    } else {
        trimmed.to_string()
    }
}

fn api_key_value_display(source: ApiKeySourceMode, value: &str) -> String {
    match source {
        ApiKeySourceMode::Text => masked_value(value),
        ApiKeySourceMode::Command | ApiKeySourceMode::Env => value.trim().to_string(),
    }
}

fn api_key_summary(source: ApiKeySourceMode, value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }

    match source {
        ApiKeySourceMode::Text => format!("{} chars", trimmed.chars().count()),
        ApiKeySourceMode::Command => "shell command".to_string(),
        ApiKeySourceMode::Env => trimmed.to_string(),
    }
}

fn badge_span(label: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!("[{label}]"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn section_header_line(title: &str, color: Color) -> Line<'static> {
    Line::from(Span::styled(
        title.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}

fn labeled_value_line(
    label: &str,
    value: &str,
    label_color: Color,
    value_color: Color,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label:<8}"),
            Style::default()
                .fg(label_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(value.to_string(), Style::default().fg(value_color)),
    ])
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::list_relative_directories;

    #[test]
    fn list_relative_directories_lists_children_for_existing_relative_dir() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("papers/nlp")).expect("create nlp dir");
        fs::create_dir_all(temp.path().join("papers/ml")).expect("create ml dir");

        let directories = list_relative_directories(temp.path(), "papers");

        assert_eq!(directories, vec!["papers/ml", "papers/nlp"]);
    }

    #[test]
    fn list_relative_directories_filters_partial_relative_path() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("papers")).expect("create papers dir");
        fs::create_dir_all(temp.path().join("reports")).expect("create reports dir");

        let directories = list_relative_directories(temp.path(), "pa");

        assert_eq!(directories, vec!["papers"]);
    }
}
