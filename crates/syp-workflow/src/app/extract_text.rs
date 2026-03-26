use crate::{
    error::{AppError, Result},
    inputs::ExtractTextRequest,
    papers::PdfCandidate,
    papers::extract::{extract_text_batch, reset_debug_extract_log},
    terminal::Verbosity,
};

/// Extracts and prints text for the provided PDFs without running the full workflow.
///
/// # Errors
/// Returns an error when arguments are invalid, extraction fails, or any file
/// cannot be processed.
pub async fn run_extract_text(args: ExtractTextRequest) -> Result<()> {
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
    let verbosity = Verbosity::new(verbose, debug, false);
    reset_debug_extract_log(debug)?;

    let candidates = args
        .files
        .iter()
        .map(|path| PdfCandidate {
            path: path.clone(),
            size_bytes: 0,
        })
        .collect::<Vec<_>>();
    let (papers, failures) = extract_text_batch(
        &candidates,
        args.page_cutoff,
        args.extractor,
        debug,
        args.pdf_extract_workers,
        verbosity,
    )
    .await;

    let failure_count = failures.len();
    for (index, paper) in papers.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("=== {} ===", paper.path.display());
        println!("file_id: {}", paper.file_id);
        println!("pages_read: {}", paper.pages_read);
        println!();
        println!("--- raw ---");
        println!("{}", paper.extracted_text);
        if verbose {
            println!();
            println!("--- llm-ready ---");
            println!("{}", paper.llm_ready_text);
        }
    }

    for (path, err) in failures {
        if !papers.is_empty() {
            println!();
        }
        eprintln!("[extract-failed] {}: {}", path.display(), err);
    }

    if failure_count > 0 {
        return Err(AppError::Execution(format!(
            "manual extraction failed for {failure_count} file(s)"
        )));
    }

    Ok(())
}
