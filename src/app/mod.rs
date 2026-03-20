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
    terminal::{self, InspectReviewPrompt, InspectReviewRequest, Verbosity},
};

const DEBUG_TUI_PROGRESS_DELAY: Duration = Duration::from_millis(200);
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

    let reviewed_categories = simulate_debug_tui_run(&debug_run, verbosity).await?;
    terminal::report::print_report(&debug_run.report, verbosity);
    terminal::report::print_category_tree(&reviewed_categories, verbosity);
    workspace.save_stage(
        RunStage::InspectOutput,
        &InspectableDebugState {
            categories: reviewed_categories,
        },
    )?;
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

async fn simulate_debug_tui_run(
    debug_run: &DebugRunData,
    verbosity: Verbosity,
) -> Result<Vec<CategoryTree>> {
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
    let reviewed_categories = simulate_debug_taxonomy_review(&debug_run.categories, verbosity)?;
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

    Ok(reviewed_categories)
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

fn simulate_debug_taxonomy_review(
    categories: &[CategoryTree],
    verbosity: Verbosity,
) -> Result<Vec<CategoryTree>> {
    let mut current_categories = categories.to_vec();
    verbosity.stage_line(
        "inspect-output",
        format!(
            "reviewing mock taxonomy with {} top-level categor(ies)",
            current_categories.len()
        ),
    );
    terminal::report::print_category_tree(&current_categories, verbosity);

    loop {
        match terminal::prompt_inspect_review_action(&current_categories, verbosity)? {
            InspectReviewPrompt::Accept => break,
            InspectReviewPrompt::Cancel => {
                return Err(AppError::Execution("inspect-output cancelled".to_string()));
            }
            InspectReviewPrompt::Suggest(request) => {
                current_categories = apply_debug_taxonomy_suggestion(&current_categories, &request);
                terminal::report::print_category_tree(&current_categories, verbosity);
                if !terminal::prompt_continue_improving()? {
                    break;
                }
            }
        }
    }

    verbosity.stage_line(
        "inspect-output",
        format!(
            "accepted mock taxonomy with {} top-level categor(ies)",
            current_categories.len()
        ),
    );
    Ok(current_categories)
}

fn apply_debug_taxonomy_suggestion(
    categories: &[CategoryTree],
    request: &InspectReviewRequest,
) -> Vec<CategoryTree> {
    let mut updated = categories.to_vec();

    for removal in &request.removals {
        let segments = removal
            .split(" > ")
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        if !segments.is_empty() {
            remove_category_path(&mut updated, &segments);
        }
    }

    if let Some(suggestion) = request
        .user_suggestion
        .as_deref()
        .map(str::trim)
        .filter(|suggestion| !suggestion.is_empty())
        && let Some(first) = updated.first_mut()
    {
        first.name = format!("{} ({suggestion})", first.name);
    }
    updated
}

fn remove_category_path(categories: &mut Vec<CategoryTree>, path: &[&str]) -> bool {
    let Some((head, tail)) = path.split_first() else {
        return false;
    };
    let Some(index) = categories
        .iter()
        .position(|category| category.name == *head)
    else {
        return false;
    };

    if tail.is_empty() {
        categories.remove(index);
        true
    } else {
        remove_category_path(&mut categories[index].children, tail)
    }
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
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    use tempfile::tempdir;

    use super::{
        apply_debug_taxonomy_suggestion, seed_debug_stages, simulate_debug_taxonomy_review,
    };
    use crate::{
        config::AppConfig,
        llm::LlmProvider,
        papers::placement::PlacementMode,
        papers::taxonomy::{CategoryTree, TaxonomyMode},
        session::{RunStage, RunWorkspace},
        terminal::{
            AlertSeverity, InspectReviewPrompt, InspectReviewRequest, TerminalBackend, Verbosity,
            install_backend,
        },
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
            use_current_folder_tree: false,
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
        let saved_review = workspace
            .load_stage::<serde_json::Value>(RunStage::InspectOutput)
            .expect("load inspect output");

        assert!(debug_run.report.dry_run);
        assert_eq!(debug_run.report.scanned, 4);
        assert_eq!(debug_run.report.processed, 3);
        assert_eq!(debug_run.report.skipped, 1);
        assert_eq!(debug_run.report.actions.len(), 3);
        assert_eq!(saved_actions.len(), 3);
        assert!(saved_review.is_none());
    }

    #[test]
    fn debug_taxonomy_suggestion_updates_first_root_label() {
        let categories = vec![
            CategoryTree {
                name: "debug".to_string(),
                children: vec![],
            },
            CategoryTree {
                name: "notes".to_string(),
                children: vec![],
            },
        ];

        let updated = apply_debug_taxonomy_suggestion(
            &categories,
            &InspectReviewRequest::from_user_suggestion("merge workflow".to_string()),
        );

        assert_eq!(updated[0].name, "debug (merge workflow)");
        assert_eq!(updated[1].name, "notes");
    }

    #[test]
    fn debug_taxonomy_review_uses_prompt_loop_and_returns_reviewed_categories() {
        let backend = Arc::new(DebugReviewBackend::new(
            vec![
                InspectReviewPrompt::Suggest(InspectReviewRequest::from_user_suggestion(
                    "merge workflow".to_string(),
                )),
                InspectReviewPrompt::Accept,
            ],
            vec![true],
        ));
        let _guard = install_backend(backend.clone());
        let categories = vec![CategoryTree {
            name: "debug".to_string(),
            children: vec![],
        }];

        let reviewed =
            simulate_debug_taxonomy_review(&categories, Verbosity::new(false, false, false))
                .expect("debug review should complete");

        assert_eq!(reviewed[0].name, "debug (merge workflow)");
        assert_eq!(*backend.inspect_calls.lock().expect("inspect calls"), 2);
        assert_eq!(*backend.continue_calls.lock().expect("continue calls"), 1);
        assert_eq!(backend.tree_renders.lock().expect("tree renders").len(), 2);
    }

    struct DebugReviewBackend {
        inspect_replies: Mutex<VecDeque<InspectReviewPrompt>>,
        continue_replies: Mutex<VecDeque<bool>>,
        tree_renders: Mutex<Vec<Vec<CategoryTree>>>,
        inspect_calls: Mutex<usize>,
        continue_calls: Mutex<usize>,
    }

    impl DebugReviewBackend {
        fn new(inspect_replies: Vec<InspectReviewPrompt>, continue_replies: Vec<bool>) -> Self {
            Self {
                inspect_replies: Mutex::new(inspect_replies.into()),
                continue_replies: Mutex::new(continue_replies.into()),
                tree_renders: Mutex::new(Vec::new()),
                inspect_calls: Mutex::new(0),
                continue_calls: Mutex::new(0),
            }
        }
    }

    impl TerminalBackend for DebugReviewBackend {
        fn stdout_is_terminal(&self) -> bool {
            false
        }

        fn stderr_is_terminal(&self) -> bool {
            false
        }

        fn supports_progress(&self) -> bool {
            false
        }

        fn is_interactive(&self) -> bool {
            true
        }

        fn write_stdout_line(&self, _line: &str) {}

        fn write_stderr_line(&self, _line: &str) {}

        fn start_progress(&self, _id: u64, _total: usize, _label: &str) {}

        fn advance_progress(&self, _id: u64, _delta: usize) {}

        fn finish_progress(&self, _id: u64) {}

        fn show_report(&self, _report: &crate::report::RunReport, _verbosity: Verbosity) {}

        fn show_category_tree(&self, categories: &[CategoryTree], _verbosity: Verbosity) {
            self.tree_renders
                .lock()
                .expect("tree renders")
                .push(categories.to_vec());
        }

        fn update_stage_status(&self, _stage: &str, _message: &str) {}

        fn record_alert(&self, _severity: AlertSeverity, _label: &str, _message: &str) {}

        fn prompt_inspect_review_action(
            &self,
            _categories: &[CategoryTree],
            _verbosity: Verbosity,
        ) -> crate::error::Result<InspectReviewPrompt> {
            *self.inspect_calls.lock().expect("inspect calls") += 1;
            self.inspect_replies
                .lock()
                .expect("inspect replies")
                .pop_front()
                .ok_or_else(|| {
                    crate::error::AppError::Execution("missing debug inspect reply".to_string())
                })
        }

        fn prompt_continue_improving(&self) -> crate::error::Result<bool> {
            *self.continue_calls.lock().expect("continue calls") += 1;
            self.continue_replies
                .lock()
                .expect("continue replies")
                .pop_front()
                .ok_or_else(|| {
                    crate::error::AppError::Execution("missing debug continue reply".to_string())
                })
        }
    }
}
