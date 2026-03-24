use std::path::PathBuf;

use crate::{
    ExtractTextArgs,
    error::{AppError, Result},
    papers::extract::{extract_text_batch, reset_debug_extract_log},
    papers::{PaperText, PdfCandidate},
    terminal,
};

pub(super) struct ExtractPreview {
    pub(super) papers: Vec<PaperText>,
    pub(super) failures: Vec<(PathBuf, String)>,
}

pub(super) async fn collect_extract_preview(args: ExtractTextArgs) -> Result<ExtractPreview> {
    if args.page_cutoff == 0 {
        return Err(AppError::Validation(
            "page_cutoff must be greater than 0".to_string(),
        ));
    }
    if args.pdf_extract_workers == 0 {
        return Err(AppError::Validation(
            "pdf_extract_workers must be greater than 0".to_string(),
        ));
    }

    let verbose = args.verbosity > 0;
    let debug = args.verbosity > 1;
    reset_debug_extract_log(debug)?;

    let candidates = args
        .files
        .iter()
        .map(|path| PdfCandidate {
            path: path.clone(),
            size_bytes: 0,
        })
        .collect::<Vec<_>>();
    let verbosity = terminal::Verbosity::new(verbose, debug, false);
    let (papers, failures) = extract_text_batch(
        &candidates,
        args.page_cutoff,
        args.extractor,
        debug,
        args.pdf_extract_workers,
        verbosity,
    )
    .await;
    Ok(ExtractPreview { papers, failures })
}

pub(super) fn render_extract_result_lines(result: &ExtractPreview) -> Vec<String> {
    let mut lines = Vec::new();
    for paper in &result.papers {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("=== {} ===", paper.path.display()));
        lines.push(format!("file_id: {}", paper.file_id));
        lines.push(format!("pages_read: {}", paper.pages_read));
        lines.push(String::new());
        lines.push("--- raw ---".to_string());
        lines.push(paper.extracted_text.clone());
        if !paper.llm_ready_text.is_empty() {
            lines.push(String::new());
            lines.push("--- llm-ready ---".to_string());
            lines.push(paper.llm_ready_text.clone());
        }
    }

    for (path, err) in &result.failures {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("[extract-failed] {}: {err}", path.display()));
    }
    if lines.is_empty() {
        lines.push("No extract output".to_string());
    }
    lines
}
