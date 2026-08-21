use std::{
    collections::hash_map::DefaultHasher,
    fs::{self, OpenOptions},
    hash::{Hash, Hasher},
    io::Write,
    path::Path,
    process::Command,
    sync::Arc,
    time::Instant,
};

use clap::ValueEnum;
use pdf_oxide::PdfDocument;
use tokio::{sync::Semaphore, task::JoinSet};

use syp_ai::llm::LlmClient;

use crate::{
    error::AppError,
    papers::{
        PaperText, PdfCandidate, preprocess::preprocess_for_llm, scanned::summarize_scanned_pdf,
    },
    terminal::{ProgressTracker, Verbosity},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum ExtractorMode {
    Auto,
    PdfOxide,
    Pdftotext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtractorUsed {
    PdfOxide,
    Pdftotext,
    PageImages,
}

const DEBUG_EXTRACT_LOG_PATH: &str = "/tmp/sortyourpapers.log";

pub fn reset_debug_extract_log(enabled: bool) -> Result<(), AppError> {
    if !enabled {
        return Ok(());
    }

    let log_path = Path::new(DEBUG_EXTRACT_LOG_PATH);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(log_path, "")?;
    eprintln!(
        "[debug][extract] writing extracted text log to {}",
        log_path.display()
    );
    Ok(())
}

/// Extracts text for every candidate.
///
/// `vision` is the model used for PDFs with no text layer; without one, those
/// PDFs are reported as failures.
pub async fn extract_text_batch(
    candidates: &[PdfCandidate],
    page_cutoff: u8,
    mode: ExtractorMode,
    debug: bool,
    workers: usize,
    verbosity: Verbosity,
    vision: Option<&Arc<dyn LlmClient>>,
) -> (Vec<PaperText>, Vec<(std::path::PathBuf, String)>) {
    let max_workers = workers.max(1);
    let semaphore = Arc::new(Semaphore::new(max_workers));
    let mut join_set = JoinSet::new();
    let mut papers = Vec::new();
    let mut failures = Vec::new();
    let mut progress = ProgressTracker::new(verbosity, candidates.len(), "preprocessing", true);

    for (index, candidate) in candidates.iter().cloned().enumerate() {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("pdf extract semaphore should not close");
        let vision = vision.cloned();
        join_set.spawn(async move {
            let _permit = permit;
            let path = candidate.path.clone();
            let file_id = make_file_id(&candidate.path);
            let result = extract_paper(
                candidate,
                page_cutoff,
                file_id,
                mode,
                debug,
                vision.as_deref(),
            )
            .await;
            (index, path, result)
        });
    }

    while let Some(join_result) = join_set.join_next().await {
        match join_result {
            Ok((index, _path, Ok(paper))) => papers.push((index, paper)),
            Ok((index, path, Err(err))) => failures.push((index, path, err.to_string())),
            Err(err) => failures.push((
                usize::MAX,
                candidates
                    .first()
                    .map(|candidate| candidate.path.clone())
                    .unwrap_or_default(),
                format!("pdf extraction join failed: {err}"),
            )),
        }
        progress.inc(1);
    }

    papers.sort_by_key(|(index, _)| *index);
    failures.sort_by_key(|(index, _, _)| *index);

    progress.finish();

    (
        papers.into_iter().map(|(_, paper)| paper).collect(),
        failures
            .into_iter()
            .map(|(_, path, reason)| (path, reason))
            .collect(),
    )
}

/// Produces the text for one PDF, reading its text layer if it has one and
/// otherwise asking the model to read the rendered pages.
async fn extract_paper(
    candidate: PdfCandidate,
    page_cutoff: u8,
    file_id: String,
    mode: ExtractorMode,
    debug: bool,
    vision: Option<&dyn LlmClient>,
) -> Result<PaperText, AppError> {
    let started = Instant::now();
    let blocking_candidate = candidate.clone();
    let layer = tokio::task::spawn_blocking(move || {
        extract_text_layer(&blocking_candidate, page_cutoff, mode)
    })
    .await
    .map_err(|err| AppError::Execution(format!("pdf extraction task failed: {err}")))??;

    if let Some(layer) = layer {
        return Ok(build_paper(&candidate, file_id, layer, started, debug));
    }

    let Some(vision) = vision else {
        return Err(AppError::Pdf(format!(
            "{} has no text layer, and no model was available to read its pages",
            candidate.path.display()
        )));
    };

    let summary = summarize_scanned_pdf(vision, &candidate.path, page_cutoff).await?;
    Ok(build_paper(
        &candidate,
        file_id,
        TextLayer {
            text: summary,
            pages_read: page_cutoff,
            extractor_used: ExtractorUsed::PageImages,
            fallback_reason: Some("no text layer; read the page images instead".to_string()),
        },
        started,
        debug,
    ))
}

pub fn extract_text_from_path(
    path: &Path,
    page_cutoff: u8,
    mode: ExtractorMode,
    debug: bool,
) -> Result<PaperText, AppError> {
    let candidate = PdfCandidate {
        path: path.to_path_buf(),
        size_bytes: 0,
    };
    let file_id = make_file_id(path);
    let started = Instant::now();
    let layer = extract_text_layer(&candidate, page_cutoff, mode)?.ok_or_else(|| {
        AppError::Pdf(format!(
            "{} has no text layer; run it through a sorting run to read the pages with a model",
            path.display()
        ))
    })?;
    Ok(build_paper(&candidate, file_id, layer, started, debug))
}

/// Text read from a PDF's own text layer.
struct TextLayer {
    text: String,
    pages_read: u8,
    extractor_used: ExtractorUsed,
    fallback_reason: Option<String>,
}

/// Reads a PDF's text layer, or reports that it has none.
///
/// `Ok(None)` means every extractor that could open the file found no text —
/// the signature of a scanned page, which the caller can hand to a model that
/// reads images instead.
fn extract_text_layer(
    candidate: &PdfCandidate,
    page_cutoff: u8,
    mode: ExtractorMode,
) -> Result<Option<TextLayer>, AppError> {
    let mut fallback_reason: Option<String> = None;
    let layer = match mode {
        ExtractorMode::Auto => match extract_with_pdf_oxide(candidate, page_cutoff) {
            Ok(Some((text, pages_read))) => Some(TextLayer {
                text,
                pages_read,
                extractor_used: ExtractorUsed::PdfOxide,
                fallback_reason: None,
            }),
            primary => {
                if let Err(primary_err) = &primary {
                    fallback_reason = Some(primary_err.to_string());
                }
                match extract_with_pdftotext(candidate, page_cutoff) {
                    Ok(Some(text)) => Some(TextLayer {
                        text,
                        pages_read: page_cutoff,
                        extractor_used: ExtractorUsed::Pdftotext,
                        fallback_reason,
                    }),
                    Ok(None) => None,
                    Err(fallback_err) => {
                        return match primary {
                            Err(primary_err) => Err(AppError::Pdf(format!(
                                "failed to extract text from {}: primary={} ; fallback={}",
                                candidate.path.display(),
                                primary_err,
                                fallback_err
                            ))),
                            // pdf_oxide opened the file and found no text, so
                            // it is readable even though poppler failed.
                            Ok(_) => Ok(None),
                        };
                    }
                }
            }
        },
        ExtractorMode::PdfOxide => {
            extract_with_pdf_oxide(candidate, page_cutoff)?.map(|(text, pages_read)| TextLayer {
                text,
                pages_read,
                extractor_used: ExtractorUsed::PdfOxide,
                fallback_reason: None,
            })
        }
        ExtractorMode::Pdftotext => {
            extract_with_pdftotext(candidate, page_cutoff)?.map(|text| TextLayer {
                text,
                pages_read: page_cutoff,
                extractor_used: ExtractorUsed::Pdftotext,
                fallback_reason: None,
            })
        }
    };

    Ok(layer)
}

fn build_paper(
    candidate: &PdfCandidate,
    file_id: String,
    layer: TextLayer,
    started: Instant,
    debug: bool,
) -> PaperText {
    let TextLayer {
        text: extracted_text,
        pages_read,
        extractor_used,
        fallback_reason,
    } = layer;
    let llm_ready_text = preprocess_for_llm(&extracted_text);

    if debug {
        let mut detail = String::new();
        if let Some(reason) = fallback_reason.as_deref() {
            detail = format!(" fallback_reason={reason}");
        }
        eprintln!(
            "[debug][extract] path={} method={} pages_read={} elapsed={}{}",
            candidate.path.display(),
            extractor_used.as_str(),
            pages_read,
            format_duration(started.elapsed()),
            detail
        );

        if let Err(err) = append_debug_extract_log(
            candidate,
            extractor_used,
            pages_read,
            started.elapsed(),
            fallback_reason.as_deref(),
            &extracted_text,
            &llm_ready_text,
        ) {
            eprintln!(
                "[debug][extract] failed to write log {}: {}",
                DEBUG_EXTRACT_LOG_PATH, err
            );
        }
    }

    PaperText {
        file_id,
        path: candidate.path.clone(),
        extracted_text,
        llm_ready_text,
        pages_read,
        from_page_images: extractor_used == ExtractorUsed::PageImages,
    }
}

/// Reads the PDF's own text layer.
///
/// `Ok(None)` means the file opened but holds no text at all, which is what a
/// scanned page looks like; that is a different situation from a PDF that
/// cannot be parsed.
fn extract_with_pdf_oxide(
    candidate: &PdfCandidate,
    page_cutoff: u8,
) -> Result<Option<(String, u8)>, AppError> {
    let mut doc = PdfDocument::open(&candidate.path)
        .map_err(|e| AppError::Pdf(format!("{}: {e}", candidate.path.display())))?;

    let pages_read = doc
        .page_count()
        .map_err(|e| {
            AppError::Pdf(format!(
                "failed to inspect {}: {e}",
                candidate.path.display()
            ))
        })?
        .min(usize::from(page_cutoff));
    if pages_read == 0 {
        return Err(AppError::Pdf(format!(
            "{} has no readable pages",
            candidate.path.display()
        )));
    }

    let mut page_text = Vec::with_capacity(pages_read);
    for page_index in 0..pages_read {
        let text = doc.extract_text(page_index).map_err(|e| {
            AppError::Pdf(format!(
                "failed to extract page {} from {}: {e}",
                page_index + 1,
                candidate.path.display()
            ))
        })?;
        if !text.trim().is_empty() {
            page_text.push(text);
        }
    }

    let extracted_text = page_text.join("\n\n");
    if extracted_text.trim().is_empty() {
        return Ok(None);
    }

    let pages_read = u8::try_from(pages_read).unwrap_or(page_cutoff);
    Ok(Some((extracted_text, pages_read)))
}

/// Reads the PDF's own text layer with poppler.
///
/// `Ok(None)` means poppler read the file and found no text.
fn extract_with_pdftotext(
    candidate: &PdfCandidate,
    page_cutoff: u8,
) -> Result<Option<String>, AppError> {
    let output = Command::new("pdftotext")
        .arg("-f")
        .arg("1")
        .arg("-l")
        .arg(page_cutoff.to_string())
        .arg("-layout")
        .arg("-q")
        .arg("-enc")
        .arg("UTF-8")
        .arg(&candidate.path)
        .arg("-")
        .output()
        .map_err(|e| AppError::Pdf(format!("pdftotext invocation failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AppError::Pdf(format!(
            "pdftotext exited with status {}{}",
            output.status,
            if stderr.is_empty() {
                "".to_string()
            } else {
                format!(": {stderr}")
            }
        )));
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return Ok(None);
    }

    Ok(Some(text))
}

pub fn make_file_id(path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    format!("paper-{:016x}", hasher.finish())
}

impl ExtractorUsed {
    fn as_str(self) -> &'static str {
        match self {
            ExtractorUsed::PdfOxide => "pdf-oxide",
            ExtractorUsed::Pdftotext => "pdftotext",
            ExtractorUsed::PageImages => "page-images",
        }
    }
}

fn format_duration(duration: std::time::Duration) -> String {
    if duration.as_secs_f64() >= 1.0 {
        format!("{:.3}s", duration.as_secs_f64())
    } else {
        format!("{:.1}ms", duration.as_secs_f64() * 1000.0)
    }
}

fn append_debug_extract_log(
    candidate: &PdfCandidate,
    extractor_used: ExtractorUsed,
    pages_read: u8,
    elapsed: std::time::Duration,
    fallback_reason: Option<&str>,
    extracted_text: &str,
    llm_ready_text: &str,
) -> Result<(), AppError> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(DEBUG_EXTRACT_LOG_PATH)?;

    writeln!(file, "=== EXTRACT ===")?;
    writeln!(file, "path: {}", candidate.path.display())?;
    writeln!(file, "method: {}", extractor_used.as_str())?;
    writeln!(file, "pages_read: {}", pages_read)?;
    writeln!(file, "elapsed: {}", format_duration(elapsed))?;
    if let Some(reason) = fallback_reason {
        writeln!(file, "fallback_reason: {}", reason)?;
    }
    writeln!(file, "--- raw text ---")?;
    writeln!(file, "{}", extracted_text)?;
    writeln!(file, "--- llm-ready text ---")?;
    writeln!(file, "{}", llm_ready_text)?;
    writeln!(file, "=== END ===")?;
    writeln!(file)?;
    file.flush()?;
    file.sync_data()?;
    Ok(())
}
