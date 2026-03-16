pub mod categorize;
pub mod config;
pub mod discovery;
pub mod error;
pub mod execute;
pub mod llm;
pub mod models;
pub mod pdf_extract;
pub mod place;
pub mod planner;
pub mod report;
pub mod text_preprocess;

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use categorize::{extract_keywords, synthesize_categories, synthesize_categories_batch_merged};
use config::{CliArgs, ExtractTextArgs};
use discovery::{dedupe_candidates, discover_pdf_candidates, split_by_size};
use error::{AppError, Result};
use llm::build_client;
use models::{AppConfig, PdfCandidate, RunReport, TaxonomyMode};
use pdf_extract::{ExtractorMode, extract_text_batch, reset_debug_extract_log};
use place::{OutputSnapshot, generate_placements, inspect_output};
use planner::build_move_plan;

pub async fn run_with_args(cli: CliArgs) -> Result<RunReport> {
    let config = config::resolve_config(cli)?;
    run(config).await
}

pub fn init_config(force: bool) -> Result<std::path::PathBuf> {
    config::init_xdg_config(force)
}

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

    reset_debug_extract_log(args.debug)?;

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
        args.debug,
        args.pdf_extract_workers,
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
        if args.debug {
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
            "manual extraction failed for {} file(s)",
            failure_count
        )));
    }

    Ok(())
}

pub async fn run(config: AppConfig) -> Result<RunReport> {
    let debug = config.debug;
    let run_started = Instant::now();
    let mut report = RunReport::new(config.dry_run);

    reset_debug_extract_log(debug)?;

    let stage_started = Instant::now();
    let mut all_candidates = discover_pdf_candidates(Path::new(&config.input), config.recursive)?;
    log_timing(debug, "discover-input", stage_started.elapsed());

    if config.rebuild && config.output.exists() {
        let stage_started = Instant::now();
        let existing = discover_pdf_candidates(Path::new(&config.output), true)?;
        log_timing(debug, "discover-output", stage_started.elapsed());
        all_candidates.extend(existing);
    }

    let stage_started = Instant::now();
    all_candidates = dedupe_candidates(all_candidates);
    log_timing(debug, "dedupe", stage_started.elapsed());
    report.scanned = all_candidates.len();

    let stage_started = Instant::now();
    let (accepted, skipped) = split_by_size(all_candidates, config.max_file_size_mb);
    log_timing(debug, "filter-size", stage_started.elapsed());
    report.skipped = skipped.len();

    let stage_started = Instant::now();
    let (papers, extraction_failures) = extract_text_batch(
        &accepted,
        config.page_cutoff,
        ExtractorMode::Auto,
        debug,
        config.pdf_extract_workers,
    )
    .await;
    log_timing(debug, "extract-text", stage_started.elapsed());
    for (path, reason) in &extraction_failures {
        eprintln!("[extract-failed] {}: {}", path.display(), reason);
    }
    report.failed += extraction_failures.len();
    report.processed = papers.len();

    if papers.is_empty() {
        report::print_report(&report);
        if report.failed > 0 {
            return Err(AppError::Execution(
                "run completed with extraction failures and no processable papers".to_string(),
            ));
        }
        log_timing(debug, "total", run_started.elapsed());
        return Ok(report);
    }

    let stage_started = Instant::now();
    let llm_client = Arc::<dyn llm::LlmClient>::from(build_client(&config));
    log_timing(debug, "build-llm-client", stage_started.elapsed());

    let stage_started = Instant::now();
    let keyword_sets =
        extract_keywords(llm_client.as_ref(), &papers, config.keyword_batch_size).await?;
    log_timing(debug, "extract-keywords", stage_started.elapsed());

    let stage_started = Instant::now();
    let categories = match config.taxonomy_mode {
        TaxonomyMode::Global => {
            synthesize_categories(llm_client.as_ref(), &keyword_sets, config.category_depth).await?
        }
        TaxonomyMode::BatchMerge => {
            synthesize_categories_batch_merged(
                Arc::clone(&llm_client),
                &papers,
                &keyword_sets,
                config.category_depth,
                config.taxonomy_batch_size,
            )
            .await?
        }
    };
    log_timing(debug, "synthesize-categories", stage_started.elapsed());

    let stage_started = Instant::now();
    let output_snapshot = inspect_output(Path::new(&config.output))?;
    log_timing(debug, "inspect-output", stage_started.elapsed());

    let placement_snapshot = pick_snapshot_for_mode(&output_snapshot, config.rebuild);

    let stage_started = Instant::now();
    let placements = generate_placements(
        llm_client.as_ref(),
        &papers,
        &keyword_sets,
        &categories,
        &placement_snapshot,
        config.placement_mode,
        config.category_depth,
    )
    .await?;
    log_timing(debug, "generate-placements", stage_started.elapsed());

    let stage_started = Instant::now();
    let actions = build_move_plan(Path::new(&config.output), &papers, &placements)?;
    log_timing(debug, "build-plan", stage_started.elapsed());
    report.actions = actions;

    let stage_started = Instant::now();
    execute::execute_plan(&report.actions, config.dry_run)?;
    log_timing(debug, "execute-plan", stage_started.elapsed());

    report::print_report(&report);
    log_timing(debug, "total", run_started.elapsed());

    if report.failed > 0 {
        return Err(AppError::Execution(
            "run completed with one or more failures".to_string(),
        ));
    }

    Ok(report)
}

fn pick_snapshot_for_mode(snapshot: &OutputSnapshot, rebuild: bool) -> OutputSnapshot {
    if rebuild {
        OutputSnapshot {
            is_empty: true,
            existing_folders: vec![".".to_string()],
            tree_map: "<rebuild-mode: ignore existing tree>".to_string(),
        }
    } else {
        snapshot.clone()
    }
}

fn log_timing(enabled: bool, stage: &str, elapsed: Duration) {
    if enabled {
        eprintln!(
            "[debug][timing] stage={stage} elapsed={}",
            format_duration(elapsed)
        );
    }
}

fn format_duration(duration: Duration) -> String {
    if duration.as_secs_f64() >= 1.0 {
        format!("{:.3}s", duration.as_secs_f64())
    } else {
        format!("{:.1}ms", duration.as_secs_f64() * 1000.0)
    }
}
