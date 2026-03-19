use std::{
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Serialize;

use crate::{
    cli::{CliArgs, ExtractTextArgs},
    config,
    config::AppConfig,
    error::{AppError, Result},
    papers::extract::{extract_text_batch, reset_debug_extract_log},
    papers::placement::PlacementDecision,
    papers::taxonomy::CategoryTree,
    papers::{
        KeywordSet, KeywordStageState, PaperText, PdfCandidate, PreliminaryCategoryPair,
        SynthesizeCategoriesState,
    },
    report::{FileAction, PlanAction, RunReport},
    session::{ExtractTextState, FilterSizeState, RunStage, RunWorkspace, run_with_workspace},
    terminal::{self, Verbosity},
};

const DEBUG_TUI_PROGRESS_DELAY: Duration = Duration::from_secs(5);
const DEBUG_TUI_PROGRESS_SETTLE_DELAY: Duration = Duration::from_millis(250);

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

pub(crate) async fn run_debug_tui(config: AppConfig) -> Result<RunReport> {
    let mut config = absolutize_config(config)?;
    config.rebuild = false;
    config.dry_run = true;

    let mut workspace = RunWorkspace::create(&config)?;
    let verbosity = Verbosity::new(config.verbose, config.debug, config.quiet);
    let debug_run = seed_debug_stages(&mut workspace, &config)?;

    simulate_debug_tui_run(&debug_run, verbosity).await;
    terminal::report::print_report(&debug_run.report, verbosity);
    terminal::report::print_category_tree(&debug_run.categories, verbosity);
    workspace.save_report(&debug_run.report)?;
    workspace.mark_completed()?;

    Ok(debug_run.report)
}

fn seed_debug_stages(workspace: &mut RunWorkspace, config: &AppConfig) -> Result<DebugRunData> {
    let candidates = vec![
        PdfCandidate {
            path: config.input.join("debug-paper-01.pdf"),
            size_bytes: 1_048_576,
        },
        PdfCandidate {
            path: config.input.join("debug-paper-02.pdf"),
            size_bytes: 512_000,
        },
        PdfCandidate {
            path: config.input.join("debug-paper-03.pdf"),
            size_bytes: 640_000,
        },
    ];

    let skipped = vec![PdfCandidate {
        path: config.input.join("debug-paper-skipped.pdf"),
        size_bytes: 99_999_999,
    }];

    let papers = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let file_id = format!("debug-paper-{index:02}");
            PaperText {
                file_id,
                path: candidate.path.clone(),
                extracted_text: "Debug extracted text from mocked PDF content".to_string(),
                llm_ready_text: "Mocked LLM-ready text for debug workflow".to_string(),
                pages_read: 5,
            }
        })
        .collect::<Vec<_>>();

    let keyword_sets = papers
        .iter()
        .map(|paper| KeywordSet {
            file_id: paper.file_id.clone(),
            keywords: vec![
                "debug".to_string(),
                "workflow".to_string(),
                "ratatui".to_string(),
            ],
        })
        .collect::<Vec<_>>();

    let preliminary_pairs = papers
        .iter()
        .map(|paper| PreliminaryCategoryPair {
            file_id: paper.file_id.clone(),
            preliminary_categories_k_depth: "debug,workflow".to_string(),
        })
        .collect::<Vec<_>>();

    let categories = vec![
        CategoryTree {
            name: "debug".to_string(),
            children: vec![CategoryTree {
                name: "workflow".to_string(),
                children: vec![],
            }],
        },
        CategoryTree {
            name: "notes".to_string(),
            children: vec![],
        },
    ];

    let placements = papers
        .iter()
        .enumerate()
        .map(|(index, paper)| PlacementDecision {
            file_id: paper.file_id.clone(),
            target_rel_path: if index == 0 {
                "debug/workflow".to_string()
            } else {
                "notes".to_string()
            },
        })
        .collect::<Vec<_>>();
    let actions = build_debug_plan_actions(&papers, &placements, &config.output);
    let report = RunReport {
        scanned: papers.len() + skipped.len(),
        processed: papers.len(),
        skipped: skipped.len(),
        failed: 0,
        actions: actions.clone(),
        dry_run: true,
        llm_usage: Default::default(),
    };

    workspace.save_stage(RunStage::DiscoverInput, &candidates)?;
    workspace.save_stage(RunStage::Dedupe, &candidates)?;
    workspace.save_stage(
        RunStage::FilterSize,
        &FilterSizeState {
            accepted: candidates,
            skipped,
        },
    )?;
    workspace.save_stage(
        RunStage::ExtractText,
        &ExtractTextState {
            papers,
            failures: Vec::new(),
        },
    )?;
    workspace.save_stage(
        RunStage::ExtractKeywords,
        &KeywordStageState {
            keyword_sets,
            preliminary_pairs,
        },
    )?;
    workspace.save_stage(
        RunStage::SynthesizeCategories,
        &SynthesizeCategoriesState {
            categories: categories.clone(),
            partial_categories: vec![categories.clone()],
        },
    )?;
    workspace.save_stage(
        RunStage::InspectOutput,
        &InspectableDebugState {
            categories: categories.clone(),
        },
    )?;
    workspace.save_stage(RunStage::GeneratePlacements, &placements)?;
    workspace.save_stage(RunStage::BuildPlan, &actions)?;
    workspace.save_report(&report)?;

    Ok(DebugRunData { categories, report })
}

fn build_debug_plan_actions(
    papers: &[PaperText],
    placements: &[PlacementDecision],
    output_root: &Path,
) -> Vec<PlanAction> {
    placements
        .iter()
        .filter_map(|placement| {
            let paper = papers
                .iter()
                .find(|candidate| candidate.file_id == placement.file_id)?;
            let filename = paper.path.file_name()?;
            Some(PlanAction {
                source: paper.path.clone(),
                destination: output_root.join(&placement.target_rel_path).join(filename),
                action: FileAction::Move,
            })
        })
        .collect()
}

async fn simulate_debug_tui_run(debug_run: &DebugRunData, verbosity: Verbosity) {
    verbosity.run_line(
        "RUN",
        format!(
            "debug_tui preview scanned {} candidate PDF(s)",
            debug_run.report.scanned
        ),
    );
    verbosity.stage_line(
        "discover-input",
        format!("found {} candidate PDF(s)", debug_run.report.scanned),
    );
    verbosity.stage_line(
        "filter-size",
        format!(
            "accepted {} PDF(s), skipped {} oversized PDF(s)",
            debug_run.report.processed, debug_run.report.skipped
        ),
    );

    let mut next_progress_id = 10_000_u64;
    simulate_progress_bar(
        &mut next_progress_id,
        "preprocessing",
        debug_run.report.processed,
    )
    .await;
    verbosity.stage_line(
        "extract-text",
        format!("extracted text for {} PDF(s)", debug_run.report.processed),
    );
    simulate_progress_bar(&mut next_progress_id, "keyword batches", 2).await;
    verbosity.stage_line("extract-keywords", "generated keyword batches".to_string());
    simulate_progress_bar(&mut next_progress_id, "taxonomy", 2).await;
    verbosity.stage_line(
        "synthesize-categories",
        format!(
            "assembled {} top-level categor(ies)",
            debug_run.categories.len()
        ),
    );
    simulate_progress_bar(&mut next_progress_id, "placement batches", 2).await;
    verbosity.stage_line(
        "generate-placements",
        format!(
            "generated {} placement decision(s)",
            debug_run.report.actions.len()
        ),
    );
    verbosity.stage_line(
        "build-plan",
        format!(
            "planned {} filesystem action(s)",
            debug_run.report.actions.len()
        ),
    );
    verbosity.stage_line(
        "execute-plan",
        "preview mode: no filesystem changes applied".to_string(),
    );
}

async fn simulate_progress_bar(next_progress_id: &mut u64, label: &str, total: usize) {
    if total == 0 {
        return;
    }

    let id = *next_progress_id;
    *next_progress_id += 1;
    terminal::current_backend().start_progress(id, total, label);

    let step_delay = DEBUG_TUI_PROGRESS_DELAY / total as u32;
    for _ in 0..total {
        tokio::time::sleep(step_delay).await;
        terminal::current_backend().advance_progress(id, 1);
    }

    tokio::time::sleep(DEBUG_TUI_PROGRESS_SETTLE_DELAY).await;
    terminal::current_backend().finish_progress(id);
}

#[derive(Debug, Serialize)]
struct InspectableDebugState {
    categories: Vec<CategoryTree>,
}

struct DebugRunData {
    categories: Vec<CategoryTree>,
    report: RunReport,
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

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::seed_debug_stages;
    use crate::{
        config::AppConfig,
        llm::LlmProvider,
        papers::placement::PlacementMode,
        papers::taxonomy::TaxonomyMode,
        session::{RunStage, RunWorkspace},
    };

    #[test]
    fn seed_debug_stages_populates_preview_report_and_build_plan() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let config = AppConfig {
            input: dir.path().join("input"),
            output: dir.path().join("output"),
            recursive: true,
            max_file_size_mb: 64,
            page_cutoff: 10,
            pdf_extract_workers: 2,
            category_depth: 2,
            taxonomy_mode: TaxonomyMode::BatchMerge,
            taxonomy_batch_size: 2,
            placement_batch_size: 2,
            placement_mode: PlacementMode::AllowNew,
            rebuild: false,
            dry_run: true,
            llm_provider: LlmProvider::Gemini,
            llm_model: "debug-model".to_string(),
            llm_base_url: None,
            api_key: None,
            keyword_batch_size: 2,
            batch_start_delay_ms: 0,
            subcategories_suggestion_number: 4,
            verbose: false,
            debug: false,
            quiet: false,
        };
        let mut workspace =
            RunWorkspace::create_with_cache_root_for_tests(dir.path(), &cache_root, &config)
                .expect("create workspace");

        let debug_run = seed_debug_stages(&mut workspace, &config).expect("seed debug stages");
        let saved_actions = workspace
            .load_stage::<Vec<crate::report::PlanAction>>(RunStage::BuildPlan)
            .expect("load build plan")
            .expect("saved build plan");

        assert!(debug_run.report.dry_run);
        assert_eq!(debug_run.report.scanned, 4);
        assert_eq!(debug_run.report.processed, 3);
        assert_eq!(debug_run.report.skipped, 1);
        assert_eq!(debug_run.report.actions.len(), 3);
        assert_eq!(saved_actions.len(), 3);
    }
}
