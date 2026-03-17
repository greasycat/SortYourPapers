pub mod categorize;
pub mod config;
pub mod discovery;
pub mod error;
pub mod execute;
pub mod llm;
pub mod logging;
pub mod models;
pub mod pdf_extract;
pub mod place;
pub mod planner;
pub mod report;
pub mod run_state;
pub mod text_preprocess;

use std::{
    env,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use categorize::{extract_keywords, synthesize_categories, synthesize_categories_batch_merged};
use config::{CliArgs, ExtractTextArgs};
use discovery::{dedupe_candidates, discover_pdf_candidates, split_by_size};
use error::{AppError, Result};
use llm::build_client;
use logging::{Verbosity, format_duration};
use models::{
    AppConfig, CategoryTree, KeywordSet, PdfCandidate, PlacementDecision, RunReport, TaxonomyMode,
};
use pdf_extract::{ExtractorMode, extract_text_batch, reset_debug_extract_log};
use place::{OutputSnapshot, PlacementOptions, generate_placements, inspect_output};
use planner::build_move_plan;
use run_state::{
    ExtractTextState, FilterSizeState, RunStage, RunSummary, RunWorkspace, StageFailure,
};

pub async fn run_with_args(cli: CliArgs) -> Result<RunReport> {
    let config = config::resolve_config(cli)?;
    run(config).await
}

pub async fn resume_run(
    run_id: Option<String>,
    apply_override: bool,
    verbosity_override: u8,
    quiet_override: bool,
) -> Result<RunReport> {
    let mut workspace = match run_id {
        Some(run_id) => RunWorkspace::open(&run_id)?,
        None => {
            let selected_run = select_run_interactively(apply_override)?;
            RunWorkspace::open(&selected_run)?
        }
    };

    if workspace.last_completed_stage() == Some(RunStage::Completed) && !apply_override {
        return Err(AppError::Execution(format!(
            "run '{}' is already completed",
            workspace.run_id()
        )));
    }

    let mut config = workspace.load_config()?;
    apply_resume_overrides(
        &mut config,
        apply_override,
        verbosity_override,
        quiet_override,
    );
    let verbosity = Verbosity::new(config.verbose, config.debug, config.quiet);
    verbosity.run_line(
        "RESUME",
        format!(
            "run_id={} mode={} state_dir={}",
            verbosity.accent(workspace.run_id()),
            if config.dry_run { "preview" } else { "apply" },
            verbosity.muted(workspace.root_dir().display().to_string())
        ),
    );
    run_with_workspace(config, &mut workspace).await
}

fn apply_resume_overrides(
    config: &mut AppConfig,
    apply_override: bool,
    verbosity_override: u8,
    quiet_override: bool,
) {
    if apply_override {
        config.dry_run = false;
    }
    if verbosity_override > 0 {
        config.verbose = true;
        config.debug = verbosity_override > 1;
    }
    if quiet_override {
        config.quiet = true;
    }
}

fn select_run_interactively(apply_override: bool) -> Result<String> {
    if !io::stdin().is_terminal() {
        return Err(AppError::Execution(
            "resume requires a RUN_ID when stdin is not interactive".to_string(),
        ));
    }

    let runs = selectable_runs(RunWorkspace::list_runs()?, apply_override);
    if runs.is_empty() {
        let runs_root = RunWorkspace::runs_root()?;
        return Err(AppError::Execution(format!(
            "no {} runs found under {}",
            if apply_override {
                "selectable"
            } else {
                "resumable"
            },
            runs_root.display()
        )));
    }

    eprintln!(
        "Available runs (resume mode: {}):",
        if apply_override { "apply" } else { "preview" }
    );
    for (index, run) in runs.iter().enumerate() {
        eprintln!("  {}", format_run_summary(index + 1, run));
    }

    let mut stderr = io::stderr();
    let mut input = String::new();
    loop {
        write!(
            stderr,
            "Choose a run to resume in {} mode by number or run id: ",
            if apply_override { "apply" } else { "preview" }
        )?;
        stderr.flush()?;

        input.clear();
        let bytes_read = io::stdin().read_line(&mut input)?;
        if bytes_read == 0 {
            return Err(AppError::Execution(
                "resume selection cancelled before a run was chosen".to_string(),
            ));
        }

        match resolve_run_selection(input.trim(), &runs) {
            Ok(run_id) => return Ok(run_id.to_string()),
            Err(err) => eprintln!("error: {err}"),
        }
    }
}

fn selectable_runs(runs: Vec<RunSummary>, apply_override: bool) -> Vec<RunSummary> {
    if apply_override {
        runs
    } else {
        runs.into_iter()
            .filter(|run| run.last_completed_stage != Some(RunStage::Completed))
            .collect()
    }
}

fn resolve_run_selection<'a>(selection: &str, runs: &'a [RunSummary]) -> Result<&'a str> {
    if selection.is_empty() {
        return Err(AppError::Execution(
            "enter a run number or run id".to_string(),
        ));
    }

    if let Ok(index) = selection.parse::<usize>() {
        let Some(run) = runs.get(index.saturating_sub(1)) else {
            return Err(AppError::Execution(format!(
                "selection '{}' is out of range",
                selection
            )));
        };
        return Ok(&run.run_id);
    }

    runs.iter()
        .find(|run| run.run_id == selection)
        .map(|run| run.run_id.as_str())
        .ok_or_else(|| AppError::Execution(format!("run '{}' was not found", selection)))
}

fn format_run_summary(index: usize, run: &RunSummary) -> String {
    let stage = run
        .last_completed_stage
        .map(|stage| format!("{stage:?}"))
        .unwrap_or_else(|| "NotStarted".to_string());
    let latest = if run.is_latest { " latest" } else { "" };
    format!(
        "{index}. {} | stage={} | cwd={} | created_unix_ms={}{}",
        run.run_id,
        stage,
        run.cwd.display(),
        run.created_unix_ms,
        latest
    )
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

    let verbose = args.verbosity > 0;
    reset_debug_extract_log(verbose)?;

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
        verbose,
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
            "manual extraction failed for {} file(s)",
            failure_count
        )));
    }

    Ok(())
}

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

async fn run_with_workspace(config: AppConfig, workspace: &mut RunWorkspace) -> Result<RunReport> {
    let verbosity = Verbosity::new(config.verbose, config.debug, config.quiet);
    let run_started = Instant::now();
    let mut report = RunReport::new(config.dry_run);

    let mut all_candidates =
        if let Some(saved) = workspace.load_stage::<Vec<PdfCandidate>>(RunStage::DiscoverInput)? {
            log_resume(verbosity, "discover-input", workspace);
            saved
        } else {
            let stage_started = Instant::now();
            log_stage(
                verbosity,
                "discover-input",
                format!(
                    "scanning {} (recursive={})",
                    config.input.display(),
                    config.recursive
                ),
            );
            let discovered = discover_pdf_candidates(Path::new(&config.input), config.recursive)?;
            workspace.save_stage(RunStage::DiscoverInput, &discovered)?;
            log_stage(
                verbosity,
                "discover-input",
                format!("found {} candidate PDF(s)", discovered.len()),
            );
            log_timing(verbosity, "discover-input", stage_started.elapsed());
            discovered
        };

    if config.rebuild && config.output.exists() {
        let existing = if let Some(saved) =
            workspace.load_stage::<Vec<PdfCandidate>>(RunStage::DiscoverOutput)?
        {
            log_resume(verbosity, "discover-output", workspace);
            saved
        } else {
            let stage_started = Instant::now();
            log_stage(
                verbosity,
                "discover-output",
                format!(
                    "rebuild mode: scanning existing output {}",
                    config.output.display()
                ),
            );
            let discovered = discover_pdf_candidates(Path::new(&config.output), true)?;
            workspace.save_stage(RunStage::DiscoverOutput, &discovered)?;
            log_stage(
                verbosity,
                "discover-output",
                format!("found {} existing output PDF(s)", discovered.len()),
            );
            log_timing(verbosity, "discover-output", stage_started.elapsed());
            discovered
        };
        all_candidates.extend(existing);
    }

    let all_candidates =
        if let Some(saved) = workspace.load_stage::<Vec<PdfCandidate>>(RunStage::Dedupe)? {
            log_resume(verbosity, "dedupe", workspace);
            saved
        } else {
            let stage_started = Instant::now();
            log_stage(
                verbosity,
                "dedupe",
                format!("deduplicating {} path(s)", all_candidates.len()),
            );
            let deduped = dedupe_candidates(all_candidates);
            workspace.save_stage(RunStage::Dedupe, &deduped)?;
            log_stage(
                verbosity,
                "dedupe",
                format!("{} unique candidate PDF(s) remain", deduped.len()),
            );
            log_timing(verbosity, "dedupe", stage_started.elapsed());
            deduped
        };
    report.scanned = all_candidates.len();

    let filter_state =
        if let Some(saved) = workspace.load_stage::<FilterSizeState>(RunStage::FilterSize)? {
            log_resume(verbosity, "filter-size", workspace);
            saved
        } else {
            let stage_started = Instant::now();
            log_stage(
                verbosity,
                "filter-size",
                format!(
                    "filtering {} candidate(s) at {}MB max",
                    all_candidates.len(),
                    config.max_file_size_mb
                ),
            );
            let (accepted, skipped) = split_by_size(all_candidates, config.max_file_size_mb);
            let state = FilterSizeState { accepted, skipped };
            workspace.save_stage(RunStage::FilterSize, &state)?;
            log_stage(
                verbosity,
                "filter-size",
                format!(
                    "accepted {} PDF(s), skipped {} oversized PDF(s)",
                    state.accepted.len(),
                    state.skipped.len()
                ),
            );
            log_timing(verbosity, "filter-size", stage_started.elapsed());
            state
        };
    report.skipped = filter_state.skipped.len();

    let extract_state =
        if let Some(saved) = workspace.load_stage::<ExtractTextState>(RunStage::ExtractText)? {
            log_resume(verbosity, "extract-text", workspace);
            saved
        } else {
            reset_debug_extract_log(verbosity.verbose_enabled())?;
            let stage_started = Instant::now();
            log_stage(
                verbosity,
                "preprocessing",
                format!(
                    "extracting text and building llm-ready terms for {} PDF(s) with {} worker(s)",
                    filter_state.accepted.len(),
                    config.pdf_extract_workers
                ),
            );
            let (papers, failures) = extract_text_batch(
                &filter_state.accepted,
                config.page_cutoff,
                ExtractorMode::Auto,
                verbosity.verbose_enabled(),
                config.pdf_extract_workers,
            )
            .await;
            let state = ExtractTextState {
                papers,
                failures: failures
                    .into_iter()
                    .map(|(path, reason)| StageFailure { path, reason })
                    .collect(),
            };
            workspace.save_stage(RunStage::ExtractText, &state)?;
            log_stage(
                verbosity,
                "preprocessing",
                format!(
                    "produced {} paper text record(s); {} extraction failure(s)",
                    state.papers.len(),
                    state.failures.len()
                ),
            );
            log_timing(verbosity, "extract-text", stage_started.elapsed());
            state
        };
    for failure in &extract_state.failures {
        verbosity.warn_line(
            "EXTRACT",
            format!("{}: {}", failure.path.display(), failure.reason),
        );
    }
    report.failed += extract_state.failures.len();
    report.processed = extract_state.papers.len();

    if extract_state.papers.is_empty() {
        if !verbosity.quiet() {
            report::print_report(&report, verbosity);
        }
        workspace.mark_completed()?;
        if report.failed > 0 {
            return Err(AppError::Execution(
                "run completed with extraction failures and no processable papers".to_string(),
            ));
        }
        log_timing(verbosity, "total", run_started.elapsed());
        return Ok(report);
    }

    let saved_keyword_sets = workspace.load_stage::<Vec<KeywordSet>>(RunStage::ExtractKeywords)?;
    let saved_categories =
        workspace.load_stage::<Vec<CategoryTree>>(RunStage::SynthesizeCategories)?;
    let saved_placements =
        workspace.load_stage::<Vec<PlacementDecision>>(RunStage::GeneratePlacements)?;
    let needs_llm =
        saved_keyword_sets.is_none() || saved_categories.is_none() || saved_placements.is_none();

    let llm_client = if needs_llm {
        let stage_started = Instant::now();
        log_stage(
            verbosity,
            "build-llm-client",
            format!(
                "provider={:?} model={}",
                config.llm_provider, config.llm_model
            ),
        );
        let client = Arc::<dyn llm::LlmClient>::from(build_client(&config));
        workspace.mark_stage(RunStage::BuildLlmClient)?;
        log_stage(verbosity, "build-llm-client", "client ready".to_string());
        log_timing(verbosity, "build-llm-client", stage_started.elapsed());
        Some(client)
    } else {
        None
    };

    let keyword_sets = if let Some(saved) = saved_keyword_sets {
        log_resume(verbosity, "extract-keywords", workspace);
        saved
    } else {
        let stage_started = Instant::now();
        let keyword_sets = extract_keywords(
            Arc::clone(
                llm_client
                    .as_ref()
                    .ok_or_else(|| AppError::Execution("missing llm client".to_string()))?,
            ),
            &extract_state.papers,
            config.keyword_batch_size,
            config.batch_start_delay_ms,
            verbosity,
        )
        .await?;
        workspace.save_stage(RunStage::ExtractKeywords, &keyword_sets)?;
        log_timing(verbosity, "extract-keywords", stage_started.elapsed());
        keyword_sets
    };

    let categories = if let Some(saved) = saved_categories {
        log_resume(verbosity, "synthesize-categories", workspace);
        saved
    } else {
        let stage_started = Instant::now();
        let categories =
            match config.taxonomy_mode {
                TaxonomyMode::Global => {
                    synthesize_categories(
                        llm_client
                            .as_deref()
                            .ok_or_else(|| AppError::Execution("missing llm client".to_string()))?,
                        &keyword_sets,
                        config.category_depth,
                        verbosity,
                    )
                    .await?
                }
                TaxonomyMode::BatchMerge => {
                    synthesize_categories_batch_merged(
                        Arc::clone(llm_client.as_ref().ok_or_else(|| {
                            AppError::Execution("missing llm client".to_string())
                        })?),
                        &extract_state.papers,
                        &keyword_sets,
                        config.category_depth,
                        config.taxonomy_batch_size,
                        config.batch_start_delay_ms,
                        verbosity,
                    )
                    .await?
                }
            };
        workspace.save_stage(RunStage::SynthesizeCategories, &categories)?;
        log_timing(verbosity, "synthesize-categories", stage_started.elapsed());
        categories
    };

    let output_snapshot =
        if let Some(saved) = workspace.load_stage::<OutputSnapshot>(RunStage::InspectOutput)? {
            log_resume(verbosity, "inspect-output", workspace);
            saved
        } else {
            let stage_started = Instant::now();
            log_stage(
                verbosity,
                "inspect-output",
                format!("reading output tree at {}", config.output.display()),
            );
            let snapshot = inspect_output(Path::new(&config.output))?;
            workspace.save_stage(RunStage::InspectOutput, &snapshot)?;
            log_stage(
                verbosity,
                "inspect-output",
                format!(
                    "output snapshot: empty={} folders={}",
                    snapshot.is_empty,
                    snapshot.existing_folders.len()
                ),
            );
            log_timing(verbosity, "inspect-output", stage_started.elapsed());
            snapshot
        };

    let placement_snapshot = pick_snapshot_for_mode(&output_snapshot, config.rebuild);

    let placements = if let Some(saved) = saved_placements {
        log_resume(verbosity, "generate-placements", workspace);
        saved
    } else {
        let stage_started = Instant::now();
        log_stage(
            verbosity,
            "generate-placements",
            format!(
                "placing {} paper(s) with mode {:?}",
                extract_state.papers.len(),
                config.placement_mode
            ),
        );
        let placements = generate_placements(
            Arc::clone(
                llm_client
                    .as_ref()
                    .ok_or_else(|| AppError::Execution("missing llm client".to_string()))?,
            ),
            &extract_state.papers,
            &keyword_sets,
            &categories,
            &placement_snapshot,
            PlacementOptions {
                batch_size: config.placement_batch_size,
                batch_start_delay_ms: config.batch_start_delay_ms,
                placement_mode: config.placement_mode,
                category_depth: config.category_depth,
                verbosity,
            },
        )
        .await?;
        workspace.save_stage(RunStage::GeneratePlacements, &placements)?;
        log_stage(
            verbosity,
            "generate-placements",
            format!("generated {} placement decision(s)", placements.len()),
        );
        log_timing(verbosity, "generate-placements", stage_started.elapsed());
        placements
    };

    let actions = if let Some(saved) = workspace.load_stage::<Vec<_>>(RunStage::BuildPlan)? {
        log_resume(verbosity, "build-plan", workspace);
        saved
    } else {
        let stage_started = Instant::now();
        log_stage(
            verbosity,
            "build-plan",
            format!("building move plan rooted at {}", config.output.display()),
        );
        let actions = build_move_plan(
            Path::new(&config.output),
            &extract_state.papers,
            &placements,
        )?;
        workspace.save_stage(RunStage::BuildPlan, &actions)?;
        log_stage(
            verbosity,
            "build-plan",
            format!("planned {} filesystem action(s)", actions.len()),
        );
        log_timing(verbosity, "build-plan", stage_started.elapsed());
        actions
    };
    report.actions = actions;

    let stage_started = Instant::now();
    log_stage(
        verbosity,
        "execute-plan",
        format!(
            "executing {} action(s) mode={}",
            report.actions.len(),
            if config.dry_run { "preview" } else { "apply" }
        ),
    );
    execute::execute_plan(&report.actions, config.dry_run)?;
    workspace.mark_stage(RunStage::ExecutePlan)?;
    log_stage(verbosity, "execute-plan", "execution complete".to_string());
    log_timing(verbosity, "execute-plan", stage_started.elapsed());

    if !verbosity.quiet() {
        report::print_report(&report, verbosity);
    }
    log_timing(verbosity, "total", run_started.elapsed());
    workspace.mark_completed()?;

    if report.failed > 0 {
        return Err(AppError::Execution(
            "run completed with one or more failures".to_string(),
        ));
    }

    Ok(report)
}

fn absolutize_config(mut config: AppConfig) -> Result<AppConfig> {
    let cwd = env::current_dir()?;
    config.input = absolutize_path(&cwd, &config.input);
    config.output = absolutize_path(&cwd, &config.output);
    Ok(config)
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

fn absolutize_path(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn log_stage(verbosity: Verbosity, stage: &str, message: String) {
    verbosity.stage_line(stage, message);
}

fn log_resume(verbosity: Verbosity, stage: &str, workspace: &RunWorkspace) {
    verbosity.debug_line(
        "RESUME",
        format!(
            "stage={} state_dir={}",
            verbosity.accent(stage),
            workspace.root_dir().display()
        ),
    );
}

fn log_timing(verbosity: Verbosity, stage: &str, elapsed: Duration) {
    if verbosity.verbose_enabled() {
        verbosity.debug_line(
            "TIMING",
            format!(
                "stage={} elapsed={}",
                verbosity.accent(stage),
                format_duration(elapsed)
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{apply_resume_overrides, resolve_run_selection, selectable_runs};
    use crate::models::{AppConfig, LlmProvider, PlacementMode, TaxonomyMode};
    use crate::run_state::{RunStage, RunSummary};

    fn sample_runs() -> Vec<RunSummary> {
        vec![
            RunSummary {
                run_id: "run-2".to_string(),
                created_unix_ms: 2,
                cwd: PathBuf::from("/tmp/two"),
                last_completed_stage: Some(RunStage::ExtractText),
                is_latest: true,
            },
            RunSummary {
                run_id: "run-3".to_string(),
                created_unix_ms: 3,
                cwd: PathBuf::from("/tmp/three"),
                last_completed_stage: Some(RunStage::Completed),
                is_latest: false,
            },
            RunSummary {
                run_id: "run-1".to_string(),
                created_unix_ms: 1,
                cwd: PathBuf::from("/tmp/one"),
                last_completed_stage: None,
                is_latest: false,
            },
        ]
    }

    #[test]
    fn resolves_run_selection_by_index() {
        let runs = selectable_runs(sample_runs(), false);

        let selected = resolve_run_selection("2", &runs).expect("resolve by index");

        assert_eq!(selected, "run-1");
    }

    #[test]
    fn resolves_run_selection_by_run_id() {
        let runs = selectable_runs(sample_runs(), false);

        let selected = resolve_run_selection("run-2", &runs).expect("resolve by id");

        assert_eq!(selected, "run-2");
    }

    #[test]
    fn rejects_invalid_run_selection() {
        let runs = selectable_runs(sample_runs(), false);

        let err = resolve_run_selection("9", &runs).expect_err("selection should fail");

        assert!(err.to_string().contains("out of range"));
    }

    #[test]
    fn filters_completed_runs_from_preview_selection() {
        let runs = selectable_runs(sample_runs(), false);

        assert_eq!(runs.len(), 2);
        assert!(
            runs.iter()
                .all(|run| run.last_completed_stage != Some(RunStage::Completed))
        );
    }

    #[test]
    fn keeps_completed_runs_in_apply_selection() {
        let runs = selectable_runs(sample_runs(), true);

        assert_eq!(runs.len(), 3);
        assert!(
            runs.iter()
                .any(|run| run.last_completed_stage == Some(RunStage::Completed))
        );
    }

    #[test]
    fn resume_apply_override_turns_off_dry_run() {
        let mut config = sample_config();

        apply_resume_overrides(&mut config, true, 0, false);

        assert!(!config.dry_run);
    }

    #[test]
    fn resume_overrides_apply_verbosity_and_quiet() {
        let mut config = sample_config();

        apply_resume_overrides(&mut config, false, 2, true);

        assert!(config.dry_run);
        assert!(config.verbose);
        assert!(config.debug);
        assert!(config.quiet);
    }

    fn sample_config() -> AppConfig {
        AppConfig {
            input: PathBuf::from("/tmp/in"),
            output: PathBuf::from("/tmp/out"),
            recursive: false,
            max_file_size_mb: 16,
            page_cutoff: 1,
            pdf_extract_workers: 8,
            category_depth: 2,
            taxonomy_mode: TaxonomyMode::BatchMerge,
            taxonomy_batch_size: 4,
            placement_batch_size: 10,
            placement_mode: PlacementMode::ExistingOnly,
            rebuild: false,
            dry_run: true,
            llm_provider: LlmProvider::Gemini,
            llm_model: "gemini-3-flash-preview".to_string(),
            llm_base_url: None,
            api_key: None,
            keyword_batch_size: 20,
            batch_start_delay_ms: 100,
            verbose: false,
            debug: false,
            quiet: false,
        }
    }
}
