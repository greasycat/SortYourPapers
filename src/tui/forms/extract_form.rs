use std::path::PathBuf;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Text},
    widgets::{ListItem, Paragraph, Wrap},
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
use crate::tui::{
    theme::ThemePalette,
    ui_widgets::{render_selectable_list, stylized_body_lines},
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
    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect, theme: ThemePalette) {
        let chunks = if area.width < 90 {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(8), Constraint::Min(0)])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
                .split(area)
        };

        let items = EXTRACT_FIELD_LABELS
            .iter()
            .enumerate()
            .map(|(index, label)| ListItem::new(format!("{label}: {}", self.value(index))))
            .collect::<Vec<_>>();
        render_selectable_list(
            frame,
            chunks[0],
            theme.block("Extract Fields"),
            items,
            Some(self.selected),
            theme,
        );

        let help = Paragraph::new(Text::from(stylized_body_lines(
            [
                "Files may be separated by commas or new lines.",
                "`Enter` edits text fields.",
                "`Left`/`Right` cycles extractor and verbosity.",
                "",
                "Press `r` to run extraction.",
            ],
            theme,
        )))
        .style(theme.panel_style())
        .wrap(Wrap { trim: false })
        .block(theme.block("Help"));
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
            verbosity: self.verbosity.raw(),
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
