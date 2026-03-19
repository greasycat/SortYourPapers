use std::path::PathBuf;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Modifier, Span, Style, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::{
    ExtractTextArgs,
    cli::{DEFAULT_PAGE_CUTOFF, DEFAULT_PDF_EXTRACT_WORKERS},
    error::{AppError, Result},
    papers::extract::ExtractorMode,
};

use super::{
    EXTRACT_FIELD_LABELS, UiVerbosity, cycle_extractor, extractor_label, parse_u8, parse_usize,
};

pub(crate) struct ExtractForm {
    pub(crate) selected: usize,
    files: String,
    page_cutoff: String,
    extractor: ExtractorMode,
    pdf_extract_workers: String,
    verbosity: UiVerbosity,
}

impl Default for ExtractForm {
    fn default() -> Self {
        Self {
            selected: 0,
            files: String::new(),
            page_cutoff: DEFAULT_PAGE_CUTOFF.to_string(),
            extractor: ExtractorMode::Auto,
            pdf_extract_workers: DEFAULT_PDF_EXTRACT_WORKERS.to_string(),
            verbosity: UiVerbosity::Normal,
        }
    }
}

impl ExtractForm {
    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);

        let lines = EXTRACT_FIELD_LABELS
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let line = format!("{label}: {}", self.value(index));
                if index == self.selected {
                    Line::from(Span::styled(
                        format!("> {line}"),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(format!("  {line}"))
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(
                Block::default()
                    .title("Extract Fields")
                    .borders(Borders::ALL),
            ),
            chunks[0],
        );

        let help = Paragraph::new(Text::from(vec![
            Line::from("Files may be separated by commas or new lines."),
            Line::from("Enter edits text fields."),
            Line::from("Left/Right cycles extractor and verbosity."),
            Line::from(""),
            Line::from("Press r to run extraction."),
        ]))
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Help").borders(Borders::ALL));
        frame.render_widget(help, chunks[1]);
    }

    pub(crate) fn apply_edit(&mut self, value: String) -> Result<()> {
        match self.selected {
            0 => self.files = value,
            1 => self.page_cutoff = value,
            3 => self.pdf_extract_workers = value,
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn build_args(&self) -> Result<ExtractTextArgs> {
        let files = self
            .files
            .split([',', '\n'])
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        if files.is_empty() {
            return Err(AppError::Validation(
                "provide at least one PDF path".to_string(),
            ));
        }
        Ok(ExtractTextArgs {
            files,
            page_cutoff: parse_u8("page_cutoff", &self.page_cutoff)?,
            extractor: self.extractor,
            pdf_extract_workers: parse_usize("pdf_extract_workers", &self.pdf_extract_workers)?,
            verbosity: self.verbosity.count(),
        })
    }

    pub(crate) fn cycle_selected(&mut self, direction: i8) {
        match self.selected {
            2 => self.extractor = cycle_extractor(self.extractor, direction),
            4 => {
                self.verbosity = if direction >= 0 {
                    self.verbosity.next()
                } else {
                    self.verbosity.previous()
                };
            }
            _ => {}
        }
    }

    pub(crate) fn value(&self, index: usize) -> String {
        match index {
            0 => self.files.clone(),
            1 => self.page_cutoff.clone(),
            2 => extractor_label(self.extractor).to_string(),
            3 => self.pdf_extract_workers.clone(),
            4 => self.verbosity.label().to_string(),
            _ => String::new(),
        }
    }
}
