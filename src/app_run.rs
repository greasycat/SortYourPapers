pub(crate) mod stages;

use std::{
    env,
    path::{Path, PathBuf},
};

use crate::{
    config::{self, CliArgs, ExtractTextArgs},
    error::{AppError, Result},
    logging::Verbosity,
    models::{AppConfig, PdfCandidate, RunReport},
    pdf_extract::{extract_text_batch, reset_debug_extract_log},
    run_state::RunWorkspace,
};

pub(crate) use stages::run_with_workspace;

/// Resolves CLI arguments into an application config and runs the main workflow.
///
/// # Errors
/// Returns an error when config resolution fails or the run itself fails.
pub async fn run_with_args(cli: CliArgs) -> Result<RunReport> {
    let config = config::resolve_config(cli)?;
    run(config).await
}

/// Runs the main PDF organization workflow using a fully resolved config.
///
/// # Errors
/// Returns an error when workspace setup fails or any pipeline stage fails.
pub async fn run(config: AppConfig) -> Result<RunReport> {
    let config = absolutize_config(config)?;
    let mut workspace = RunWorkspace::create(&config)?;
    let verbosity = Verbosity::new(config.verbose, config.debug, config.quiet);
    verbosity.run_line(
        "RUN",
        format!(
            "run_id={} state_dir={}",
            verbosity.accent(workspace.run_id()),
            verbosity.muted(workspace.root_dir().display().to_string())
        ),
    );
    run_with_workspace(config, &mut workspace).await
}

/// Extracts and prints text for the provided PDFs without running the full workflow.
///
/// # Errors
/// Returns an error when arguments are invalid, extraction fails, or any file
/// cannot be processed.
pub async fn run_extract_text(args: ExtractTextArgs) -> Result<()> {
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

fn absolutize_config(mut config: AppConfig) -> Result<AppConfig> {
    let cwd = env::current_dir()?;
    config.input = absolutize_path(&cwd, &config.input);
    config.output = absolutize_path(&cwd, &config.output);
    Ok(config)
}

fn absolutize_path(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}
