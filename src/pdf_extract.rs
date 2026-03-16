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

use crate::{
    error::AppError,
    models::{PaperText, PdfCandidate},
    text_preprocess::preprocess_for_llm,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum ExtractorMode {
    Auto,
    #[value(alias = "lopdf")]
    PdfOxide,
    Pdftotext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtractorUsed {
    PdfOxide,
    Pdftotext,
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

pub async fn extract_text_batch(
    candidates: &[PdfCandidate],
    page_cutoff: u8,
    mode: ExtractorMode,
    debug: bool,
    workers: usize,
) -> (Vec<PaperText>, Vec<(std::path::PathBuf, String)>) {
    let max_workers = workers.max(1);
    let semaphore = Arc::new(Semaphore::new(max_workers));
    let mut join_set = JoinSet::new();
    let mut papers = Vec::new();
    let mut failures = Vec::new();

    for (index, candidate) in candidates.iter().cloned().enumerate() {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("pdf extract semaphore should not close");
        join_set.spawn(async move {
            let _permit = permit;
            let path = candidate.path.clone();
            let file_id = make_file_id(&candidate.path);
            let result = tokio::task::spawn_blocking(move || {
                extract_one(&candidate, page_cutoff, file_id, mode, debug)
            })
            .await
            .map_err(|err| AppError::Execution(format!("pdf extraction task failed: {err}")))
            .and_then(|result| result);
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
    }

    papers.sort_by_key(|(index, _)| *index);
    failures.sort_by_key(|(index, _, _)| *index);

    (
        papers.into_iter().map(|(_, paper)| paper).collect(),
        failures
            .into_iter()
            .map(|(_, path, reason)| (path, reason))
            .collect(),
    )
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
    extract_one(&candidate, page_cutoff, file_id, mode, debug)
}

fn extract_one(
    candidate: &PdfCandidate,
    page_cutoff: u8,
    file_id: String,
    mode: ExtractorMode,
    debug: bool,
) -> Result<PaperText, AppError> {
    let started = Instant::now();

    let mut fallback_reason: Option<String> = None;
    let (extracted_text, pages_read, extractor_used) = match mode {
        ExtractorMode::Auto => match extract_with_pdf_oxide(candidate, page_cutoff) {
            Ok((text, pages_read)) => (text, pages_read, ExtractorUsed::PdfOxide),
            Err(primary_err) => {
                fallback_reason = Some(primary_err.to_string());
                match extract_with_pdftotext(candidate, page_cutoff) {
                    Ok(text) => (text, page_cutoff, ExtractorUsed::Pdftotext),
                    Err(fallback_err) => {
                        return Err(AppError::Pdf(format!(
                            "failed to extract text from {}: primary={} ; fallback={}",
                            candidate.path.display(),
                            primary_err,
                            fallback_err
                        )));
                    }
                }
            }
        },
        ExtractorMode::PdfOxide => {
            let (text, pages_read) = extract_with_pdf_oxide(candidate, page_cutoff)?;
            (text, pages_read, ExtractorUsed::PdfOxide)
        }
        ExtractorMode::Pdftotext => {
            let text = extract_with_pdftotext(candidate, page_cutoff)?;
            (text, page_cutoff, ExtractorUsed::Pdftotext)
        }
    };

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

    Ok(PaperText {
        file_id,
        path: candidate.path.clone(),
        extracted_text,
        llm_ready_text,
        pages_read,
    })
}

fn extract_with_pdf_oxide(
    candidate: &PdfCandidate,
    page_cutoff: u8,
) -> Result<(String, u8), AppError> {
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
        return Err(AppError::Pdf(format!(
            "pdf_oxide produced empty output for {}",
            candidate.path.display()
        )));
    }

    let pages_read = u8::try_from(pages_read).unwrap_or(page_cutoff);
    Ok((extracted_text, pages_read))
}

fn extract_with_pdftotext(candidate: &PdfCandidate, page_cutoff: u8) -> Result<String, AppError> {
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
        return Err(AppError::Pdf("pdftotext produced empty output".to_string()));
    }

    Ok(text)
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
