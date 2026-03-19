use std::path::PathBuf;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Modifier, Span, Style, Text},
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
    RUN_FIELD_LABELS, UiVerbosity, bool_label, cycle_placement_mode, cycle_provider,
    cycle_taxonomy_mode, empty_string_to_option, masked_value, parse_u8, parse_u64, parse_usize,
    placement_mode_label, provider_label, taxonomy_mode_label,
};

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
    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);

        let block = Block::default()
            .title("Run Configuration")
            .borders(Borders::ALL);
        let inner = block.inner(chunks[0]);
        let (lines, selected_line) = self.sectioned_lines();
        let scroll = selected_line.saturating_sub(usize::from(inner.height.saturating_sub(1)));
        frame.render_widget(
            Paragraph::new(lines)
                .scroll((scroll as u16, 0))
                .wrap(Wrap { trim: false })
                .block(block),
            chunks[0],
        );

        let help = Paragraph::new(Text::from(vec![
            Line::from("Enter edits text/number/path fields."),
            Line::from("Left/Right cycles enum fields."),
            Line::from("Space toggles booleans."),
            Line::from(""),
            Line::from(format!(
                "mode: {}",
                if self.apply { "apply" } else { "preview" }
            )),
            Line::from(format!("verbosity: {}", self.verbosity.label())),
            Line::from(""),
            Line::from("Press r to start the run."),
        ]))
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Help").borders(Borders::ALL));
        frame.render_widget(help, chunks[1]);
    }

    pub(crate) fn build_config(&self) -> Result<AppConfig> {
        let cli = CliArgs {
            input: Some(PathBuf::from(self.input.clone())),
            output: Some(PathBuf::from(self.output.clone())),
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
            llm_model: Some(self.llm_model.clone()),
            llm_base_url: empty_string_to_option(&self.llm_base_url),
            api_key: empty_string_to_option(&self.api_key),
            keyword_batch_size: Some(parse_usize("keyword_batch_size", &self.keyword_batch_size)?),
            subcategories_suggestion_number: Some(parse_usize(
                "subcategories_suggestion_number",
                &self.subcategories_suggestion_number,
            )?),
            verbosity: self.verbosity.count(),
            quiet: self.quiet,
        };
        config::resolve_config(cli)
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

    fn sectioned_lines(&self) -> (Vec<Line<'static>>, usize) {
        const FIELD_SECTIONS: [(&str, &[usize]); 6] = [
            ("Paths & Scope", &[0, 1, 2]),
            ("Extraction", &[3, 4, 5]),
            ("Taxonomy", &[6, 7, 8, 17, 18]),
            ("Placement & Run", &[9, 10, 11, 12]),
            ("LLM & API", &[13, 14, 15, 16]),
            ("Output & Logs", &[19, 20]),
        ];

        let mut lines = Vec::new();
        let mut selected_line = 0;
        for (section_index, (title, fields)) in FIELD_SECTIONS.iter().enumerate() {
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
                let label = RUN_FIELD_LABELS[*field_index];
                let line = format!("{label}: {}", self.value(*field_index));
                if *field_index == self.selected {
                    selected_line = lines.len();
                    lines.push(Line::from(Span::styled(
                        format!("> {line}"),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                } else {
                    lines.push(Line::from(format!("  {line}")));
                }
            }
        }

        (lines, selected_line)
    }
}
