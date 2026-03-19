use std::{
    fs,
    path::{Path, PathBuf},
};

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Modifier, Span, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
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
    UiVerbosity, bool_label, cycle_placement_mode, cycle_provider, cycle_taxonomy_mode,
    empty_string_to_option, masked_value, parse_u8, parse_u64, parse_usize, placement_mode_label,
    provider_label, run_field_help, run_field_key, run_field_label, taxonomy_mode_label,
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
    placement_batch_size: String,
    pub(crate) placement_mode: PlacementMode,
    pub(crate) rebuild: bool,
    pub(crate) apply: bool,
    llm_provider: LlmProvider,
    llm_model: String,
    llm_base_url: String,
    api_key: String,
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
            placement_batch_size: DEFAULT_PLACEMENT_BATCH_SIZE.to_string(),
            placement_mode: PlacementMode::ExistingOnly,
            rebuild: false,
            apply: false,
            llm_provider: DEFAULT_LLM_PROVIDER,
            llm_model: DEFAULT_LLM_MODEL.to_string(),
            llm_base_url: String::new(),
            api_key: String::new(),
            keyword_batch_size: DEFAULT_KEYWORD_BATCH_SIZE.to_string(),
            subcategories_suggestion_number: DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER.to_string(),
            verbosity: UiVerbosity::Normal,
            quiet: false,
        }
    }
}

impl RunForm {
    const COLUMN_FIELDS: [&'static [usize]; 3] = [
        &[0, 1, 2, 3, 4, 5],
        &[6, 7, 8, 17, 18, 9, 10],
        &[13, 14, 15, 16, 11, 12, 19, 20],
    ];

    const VISIBLE_FIELDS: [usize; 21] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 17, 18, 9, 10, 13, 14, 15, 16, 11, 12, 19, 20,
    ];

    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect) {
        let analysis = self.analysis();
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(64), Constraint::Percentage(36)])
            .split(area);
        let side_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
            .split(chunks[1]);

        self.draw_form_workspace(frame, chunks[0], &analysis);
        self.draw_summary(frame, side_chunks[0], &analysis);
        self.draw_selected_field(frame, side_chunks[1], &analysis);
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
        self.validate_numeric_field(17, &self.keyword_batch_size, parse_usize, &mut issues);
        self.validate_numeric_field(
            18,
            &self.subcategories_suggestion_number,
            parse_usize,
            &mut issues,
        );

        if matches!(self.llm_provider, LlmProvider::Openai | LlmProvider::Gemini)
            && self.api_key.trim().is_empty()
        {
            issues.push(ValidationIssue {
                field: Some(16),
                severity: ValidationSeverity::Warning,
                message: format!(
                    "{} usually requires an API key unless credentials are supplied elsewhere.",
                    provider_label(self.llm_provider)
                ),
            });
        }

        if self.quiet && !matches!(self.verbosity, UiVerbosity::Normal) {
            issues.push(ValidationIssue {
                field: Some(20),
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
            taxonomy_batch_size: Some(parse_usize(
                "taxonomy_batch_size",
                &self.taxonomy_batch_size,
            )?),
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
            api_key: empty_string_to_option(&self.api_key),
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
        !matches!(index, 2 | 7 | 10 | 11 | 12 | 13 | 19 | 20)
    }

    pub(crate) fn toggle_selected(&mut self) {
        match self.selected {
            2 => self.recursive = !self.recursive,
            11 => self.rebuild = !self.rebuild,
            12 => self.apply = !self.apply,
            20 => self.quiet = !self.quiet,
            _ => self.cycle_selected(1),
        }
    }

    pub(crate) fn cycle_selected(&mut self, direction: i8) {
        match self.selected {
            7 => self.taxonomy_mode = cycle_taxonomy_mode(self.taxonomy_mode, direction),
            10 => self.placement_mode = cycle_placement_mode(self.placement_mode, direction),
            13 => self.llm_provider = cycle_provider(self.llm_provider, direction),
            19 => {
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
            16 => self.api_key = value,
            17 => self.keyword_batch_size = value,
            18 => self.subcategories_suggestion_number = value,
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
            16 => masked_value(&self.api_key),
            17 => self.keyword_batch_size.clone(),
            18 => self.subcategories_suggestion_number.clone(),
            19 => self.verbosity.label().to_string(),
            20 => bool_label(self.quiet).to_string(),
            _ => String::new(),
        }
    }

    fn draw_form_workspace(&self, frame: &mut Frame, area: Rect, analysis: &RunFormAnalysis) {
        let outer = Block::default().title("Run Setup").borders(Borders::ALL);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(34),
                Constraint::Percentage(33),
                Constraint::Percentage(33),
            ])
            .split(inner);

        const COLUMN_SECTIONS: [[(&str, &[usize]); 2]; 3] = [
            [("Paths & Scope", &[0, 1, 2]), ("Extraction", &[3, 4, 5])],
            [("Taxonomy", &[6, 7, 8, 17, 18]), ("Placement", &[9, 10])],
            [("LLM & API", &[13, 14, 15, 16]), ("Run", &[11, 12, 19, 20])],
        ];

        for (column, sections) in chunks.iter().zip(COLUMN_SECTIONS.iter()) {
            self.draw_column(frame, *column, sections, analysis);
        }
    }

    fn draw_summary(&self, frame: &mut Frame, area: Rect, analysis: &RunFormAnalysis) {
        let mut lines = Vec::new();
        let (infos, warnings, errors) = analysis.issue_counts();
        let readiness = if analysis.has_errors() {
            Span::styled(
                analysis.readiness_text(),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                analysis.readiness_text(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        };

        lines.push(Line::from(readiness));
        lines.push(Line::from(format!(
            "Issues: {errors} error(s), {warnings} warning(s), {infos} note(s)"
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(format!(
            "Mode: {}",
            if self.apply {
                "Apply moves"
            } else {
                "Preview only"
            }
        )));
        lines.push(Line::from(format!(
            "Input: {}",
            display_path_line(&self.input, DEFAULT_INPUT)
        )));
        lines.push(Line::from(format!(
            "Output: {}",
            display_path_line(&self.output, DEFAULT_OUTPUT)
        )));
        lines.push(Line::from(format!(
            "Scope: {} | {} page(s) | {} worker(s)",
            if self.recursive {
                "recursive"
            } else {
                "top-level only"
            },
            self.page_cutoff.trim(),
            self.pdf_extract_workers.trim()
        )));
        lines.push(Line::from(format!(
            "Taxonomy: depth {} | {} | batch {}",
            self.category_depth.trim(),
            taxonomy_mode_label(self.taxonomy_mode),
            self.taxonomy_batch_size.trim()
        )));
        lines.push(Line::from(format!(
            "Placement: {} | batch {}",
            placement_mode_label(self.placement_mode),
            self.placement_batch_size.trim()
        )));
        lines.push(Line::from(format!(
            "LLM: {} / {}",
            provider_label(self.llm_provider),
            if self.llm_model.trim().is_empty() {
                "<missing>"
            } else {
                self.llm_model.trim()
            }
        )));
        lines.push(Line::from(format!(
            "Run Output: rebuild={} | quiet={} | verbosity={}",
            bool_label(self.rebuild),
            bool_label(self.quiet),
            self.verbosity.label()
        )));

        if let Some(config) = &analysis.config {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Resolved Launch",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(format!(
                "{} | provider={} | model={}",
                if config.dry_run { "preview" } else { "apply" },
                provider_label(config.llm_provider),
                config.llm_model
            )));
        }

        let notable_issues = analysis.issues.iter().take(4).collect::<Vec<_>>();
        if !notable_issues.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Notable Issues",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
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
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(
                Block::default()
                    .title("Launch Preview")
                    .borders(Borders::ALL),
            ),
            area,
        );
    }

    fn draw_selected_field(&self, frame: &mut Frame, area: Rect, analysis: &RunFormAnalysis) {
        let selected_label = run_field_label(self.selected);
        let selected_value = self.value(self.selected);
        let mut lines = vec![
            Line::from(Span::styled(
                selected_label,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(run_field_help(self.selected)),
            Line::from(""),
            Line::from(format!("Current value: {selected_value}")),
        ];

        if let Some(issue) = analysis.field_issue(self.selected) {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("{}: {}", issue.severity.title(), issue.message),
                Style::default()
                    .fg(issue.severity.color())
                    .add_modifier(Modifier::BOLD),
            )));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Controls",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from("Enter edit / toggle"));
        lines.push(Line::from("Space cycle toggle fields"));
        lines.push(Line::from("r start run when ready"));
        lines.push(Line::from("Esc back"));

        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(
                Block::default()
                    .title("Selected Field")
                    .borders(Borders::ALL),
            ),
            area,
        );
    }

    fn draw_column(
        &self,
        frame: &mut Frame,
        area: Rect,
        sections: &[(&str, &[usize])],
        analysis: &RunFormAnalysis,
    ) {
        let mut lines = Vec::new();
        let mut selected_line = 0;
        for (section_index, (title, fields)) in sections.iter().enumerate() {
            if section_index > 0 {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                (*title).to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));

            for field_index in *fields {
                let marker = analysis
                    .field_issue(*field_index)
                    .map_or(' ', |issue| issue.severity.marker());
                let line = format!(
                    "{} {}: {}",
                    marker,
                    run_field_label(*field_index),
                    self.value(*field_index)
                );

                if *field_index == self.selected {
                    selected_line = lines.len();
                    lines.push(Line::from(Span::styled(
                        format!("> {line}"),
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )));
                } else if let Some(issue) = analysis.field_issue(*field_index) {
                    lines.push(Line::from(Span::styled(
                        format!("  {line}"),
                        Style::default().fg(issue.severity.color()),
                    )));
                } else {
                    lines.push(Line::from(format!("  {line}")));
                }
            }
        }

        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(area);
        let scroll = selected_line.saturating_sub(usize::from(inner.height.saturating_sub(1)));
        frame.render_widget(
            Paragraph::new(lines)
                .scroll((scroll as u16, 0))
                .wrap(Wrap { trim: false })
                .block(block),
            area,
        );
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
