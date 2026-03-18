use std::{path::Path, sync::Arc, time::Instant};

use crate::{
    categorize::{
        KeywordBatchProgress, TaxonomyBatchProgress, extract_keywords_with_progress,
        merge_category_batches, synthesize_category_batches_with_progress,
    },
    discovery::{dedupe_candidates, discover_pdf_candidates, split_by_size},
    error::{AppError, Result},
    llm::{self, build_client},
    logging::Verbosity,
    models::{
        AppConfig, KeywordStageState, PdfCandidate, PlacementDecision, RunReport,
        SynthesizeCategoriesState,
    },
    pdf_extract::{ExtractorMode, extract_text_batch, reset_debug_extract_log},
    report,
    run_state::{ExtractTextState, FilterSizeState, RunStage, RunWorkspace, StageFailure},
};

use super::{
    output_flow::{
        build_plan_stage, execute_plan_stage, finalize_empty_run, generate_placements_stage,
        inspect_output_stage,
    },
    planning::{StagePlan, log_resume, log_stage, log_timing},
};

const KEYWORD_BATCH_PROGRESS_FILE: &str = "06-extract-keywords-partial-batches.json";
const TAXONOMY_BATCH_PROGRESS_FILE: &str = "07-synthesize-categories-partial-batches.json";

pub(crate) async fn run_with_workspace(
    config: AppConfig,
    workspace: &mut RunWorkspace,
) -> Result<RunReport> {
    let verbosity = Verbosity::new(config.verbose, config.debug, config.quiet);
    let run_started = Instant::now();
    let mut report = workspace
        .load_report()?
        .unwrap_or_else(|| RunReport::new(config.dry_run));
    report.dry_run = config.dry_run;
    let saved_keyword_state =
        workspace.load_stage::<KeywordStageState>(RunStage::ExtractKeywords)?;
    let saved_categories =
        workspace.load_stage::<SynthesizeCategoriesState>(RunStage::SynthesizeCategories)?;
    let saved_placements =
        workspace.load_stage::<Vec<PlacementDecision>>(RunStage::GeneratePlacements)?;
    let needs_llm =
        saved_keyword_state.is_none() || saved_categories.is_none() || saved_placements.is_none();
    let stage_plan = StagePlan::new(&config, needs_llm);

    let mut all_candidates = discover_input_stage(&config, workspace, verbosity, &stage_plan)?;
    if let Some(existing) = discover_output_stage(&config, workspace, verbosity, &stage_plan)? {
        all_candidates.extend(existing);
    }

    let all_candidates = dedupe_stage(all_candidates, workspace, verbosity, &stage_plan)?;
    report.scanned = all_candidates.len();
    workspace.save_report(&report)?;

    let filter_state =
        filter_size_stage(all_candidates, &config, workspace, verbosity, &stage_plan)?;
    report.skipped = filter_state.skipped.len();
    workspace.save_report(&report)?;

    let extract_state =
        extract_text_stage(&filter_state, &config, workspace, verbosity, &stage_plan).await?;
    for failure in &extract_state.failures {
        verbosity.warn_line(
            "EXTRACT",
            format!("{}: {}", failure.path.display(), failure.reason),
        );
    }
    report.failed += extract_state.failures.len();
    report.processed = extract_state.papers.len();
    workspace.save_report(&report)?;

    if extract_state.papers.is_empty() {
        return finalize_empty_run(report, workspace, verbosity, run_started.elapsed());
    }

    let llm_client = build_llm_client_stage(&config, needs_llm, workspace, verbosity, &stage_plan)?;
    let keyword_sets = extract_keywords_stage(
        saved_keyword_state,
        llm_client.as_ref(),
        &extract_state,
        &config,
        &mut report,
        workspace,
        verbosity,
        &stage_plan,
    )
    .await?;
    let categories_state = if let Some(saved) = saved_categories {
        workspace.remove_artifact(TAXONOMY_BATCH_PROGRESS_FILE)?;
        saved
    } else {
        synthesize_categories_stage(
            llm_client.as_ref(),
            &keyword_sets,
            &config,
            &mut report,
            workspace,
            verbosity,
            &stage_plan,
        )
        .await?
    };
    let categories = inspect_output_stage(
        &categories_state,
        llm_client.as_ref(),
        &config,
        &mut report,
        workspace,
        verbosity,
        &stage_plan,
    )
    .await?;
    let placements = generate_placements_stage(
        saved_placements,
        llm_client.as_ref(),
        &extract_state,
        &keyword_sets.keyword_sets,
        &keyword_sets.preliminary_pairs,
        &categories,
        &config,
        &mut report,
        workspace,
        verbosity,
        &stage_plan,
    )
    .await?;
    let actions = build_plan_stage(
        &extract_state,
        &placements,
        &config,
        workspace,
        verbosity,
        &stage_plan,
    )?;
    report.actions = actions;
    workspace.save_report(&report)?;

    execute_plan_stage(&report, &config, workspace, verbosity, &stage_plan)?;

    if !verbosity.quiet() {
        report::print_report(&report, verbosity);
        report::print_category_tree(&categories, verbosity);
    }
    log_timing(verbosity, "total", run_started.elapsed());
    workspace.save_report(&report)?;
    workspace.mark_completed()?;

    if report.failed > 0 {
        return Err(AppError::Execution(
            "run completed with one or more failures".to_string(),
        ));
    }

    Ok(report)
}

fn discover_input_stage(
    config: &AppConfig,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
) -> Result<Vec<PdfCandidate>> {
    stage_plan.announce(verbosity, RunStage::DiscoverInput);
    if let Some(saved) = workspace.load_stage::<Vec<PdfCandidate>>(RunStage::DiscoverInput)? {
        log_resume(verbosity, "discover-input", workspace);
        return Ok(saved);
    }

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
    Ok(discovered)
}

fn discover_output_stage(
    config: &AppConfig,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
) -> Result<Option<Vec<PdfCandidate>>> {
    if !(config.rebuild && config.output.exists()) {
        return Ok(None);
    }

    stage_plan.announce(verbosity, RunStage::DiscoverOutput);
    if let Some(saved) = workspace.load_stage::<Vec<PdfCandidate>>(RunStage::DiscoverOutput)? {
        log_resume(verbosity, "discover-output", workspace);
        return Ok(Some(saved));
    }

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
    Ok(Some(discovered))
}

fn dedupe_stage(
    all_candidates: Vec<PdfCandidate>,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
) -> Result<Vec<PdfCandidate>> {
    stage_plan.announce(verbosity, RunStage::Dedupe);
    if let Some(saved) = workspace.load_stage::<Vec<PdfCandidate>>(RunStage::Dedupe)? {
        log_resume(verbosity, "dedupe", workspace);
        return Ok(saved);
    }

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
    Ok(deduped)
}

fn filter_size_stage(
    all_candidates: Vec<PdfCandidate>,
    config: &AppConfig,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
) -> Result<FilterSizeState> {
    stage_plan.announce(verbosity, RunStage::FilterSize);
    if let Some(saved) = workspace.load_stage::<FilterSizeState>(RunStage::FilterSize)? {
        log_resume(verbosity, "filter-size", workspace);
        return Ok(saved);
    }

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
    Ok(state)
}

async fn extract_text_stage(
    filter_state: &FilterSizeState,
    config: &AppConfig,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
) -> Result<ExtractTextState> {
    stage_plan.announce(verbosity, RunStage::ExtractText);
    if let Some(saved) = workspace.load_stage::<ExtractTextState>(RunStage::ExtractText)? {
        log_resume(verbosity, "extract-text", workspace);
        return Ok(saved);
    }

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
        verbosity,
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
    Ok(state)
}

fn build_llm_client_stage(
    config: &AppConfig,
    needs_llm: bool,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
) -> Result<Option<Arc<dyn llm::LlmClient>>> {
    if !needs_llm {
        return Ok(None);
    }

    stage_plan.announce(verbosity, RunStage::BuildLlmClient);
    let stage_started = Instant::now();
    log_stage(
        verbosity,
        "build-llm-client",
        format!(
            "provider={:?} model={}",
            config.llm_provider, config.llm_model
        ),
    );
    let client = Arc::<dyn llm::LlmClient>::from(build_client(config));
    workspace.mark_stage(RunStage::BuildLlmClient)?;
    log_stage(verbosity, "build-llm-client", "client ready".to_string());
    log_timing(verbosity, "build-llm-client", stage_started.elapsed());
    Ok(Some(client))
}

async fn extract_keywords_stage(
    saved_keyword_state: Option<KeywordStageState>,
    llm_client: Option<&Arc<dyn llm::LlmClient>>,
    extract_state: &ExtractTextState,
    config: &AppConfig,
    report: &mut RunReport,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
) -> Result<KeywordStageState> {
    stage_plan.announce(verbosity, RunStage::ExtractKeywords);
    if let Some(saved) = saved_keyword_state {
        if saved.is_complete() {
            workspace.remove_artifact(KEYWORD_BATCH_PROGRESS_FILE)?;
            log_resume(verbosity, "extract-keywords", workspace);
            return Ok(saved);
        }

        log_stage(
            verbosity,
            "extract-keywords",
            "saved keyword state is missing preliminary categories; rerunning keyword extraction"
                .to_string(),
        );
    }

    let stage_started = Instant::now();
    let saved_progress = workspace
        .load_artifact::<KeywordBatchProgress>(KEYWORD_BATCH_PROGRESS_FILE)?
        .unwrap_or_default();
    if !saved_progress.completed_batches.is_empty() {
        report.llm_usage.keywords = saved_progress.usage.clone();
        workspace.save_report(report)?;
    }
    let (keyword_state, usage) = extract_keywords_with_progress(
        Arc::clone(require_llm_client(llm_client)?),
        &extract_state.papers,
        config.keyword_batch_size,
        config.batch_start_delay_ms,
        saved_progress,
        |progress| {
            report.llm_usage.keywords = progress.usage.clone();
            workspace.save_artifact(KEYWORD_BATCH_PROGRESS_FILE, progress)?;
            workspace.save_report(report)
        },
        verbosity,
    )
    .await?;
    report.llm_usage.keywords = usage;
    workspace.save_stage(RunStage::ExtractKeywords, &keyword_state)?;
    workspace.remove_artifact(KEYWORD_BATCH_PROGRESS_FILE)?;
    workspace.save_report(report)?;
    log_timing(verbosity, "extract-keywords", stage_started.elapsed());
    Ok(keyword_state)
}

#[allow(clippy::too_many_arguments)]
async fn synthesize_categories_stage(
    llm_client: Option<&Arc<dyn llm::LlmClient>>,
    keyword_state: &KeywordStageState,
    config: &AppConfig,
    report: &mut RunReport,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
) -> Result<SynthesizeCategoriesState> {
    stage_plan.announce(verbosity, RunStage::SynthesizeCategories);
    let stage_started = Instant::now();
    let saved_progress = workspace
        .load_artifact::<TaxonomyBatchProgress>(TAXONOMY_BATCH_PROGRESS_FILE)?
        .unwrap_or_default();
    if !saved_progress.completed_batches.is_empty() {
        report.llm_usage.taxonomy = saved_progress.usage.clone();
        workspace.save_report(report)?;
    }
    let batch_progress = synthesize_category_batches_with_progress(
        require_llm_client(llm_client)?.as_ref(),
        &keyword_state.preliminary_pairs,
        config.category_depth,
        config.taxonomy_batch_size,
        config.batch_start_delay_ms,
        saved_progress,
        |progress| {
            report.llm_usage.taxonomy = progress.usage.clone();
            workspace.save_artifact(TAXONOMY_BATCH_PROGRESS_FILE, progress)?;
            workspace.save_report(report)
        },
        verbosity,
    )
    .await?;
    let partial_categories = batch_progress
        .completed_batches
        .iter()
        .map(|batch| batch.categories.clone())
        .collect::<Vec<_>>();
    let (categories, merge_usage) = merge_category_batches(
        require_llm_client(llm_client)?.as_ref(),
        &partial_categories,
        config.category_depth,
        None,
        verbosity,
    )
    .await?;
    let mut usage = batch_progress.usage;
    usage.merge(&merge_usage);
    report.llm_usage.taxonomy = usage;
    let state = SynthesizeCategoriesState {
        categories,
        partial_categories,
    };
    workspace.save_stage(RunStage::SynthesizeCategories, &state)?;
    workspace.remove_artifact(TAXONOMY_BATCH_PROGRESS_FILE)?;
    workspace.save_report(report)?;
    log_timing(verbosity, "synthesize-categories", stage_started.elapsed());
    Ok(state)
}

fn require_llm_client(
    client: Option<&Arc<dyn llm::LlmClient>>,
) -> Result<&Arc<dyn llm::LlmClient>> {
    client.ok_or_else(|| AppError::Execution("missing llm client".to_string()))
}
