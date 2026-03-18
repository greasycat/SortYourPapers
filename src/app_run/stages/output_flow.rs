use std::{
    future::Future,
    io::{self, BufRead, IsTerminal, Write},
    path::Path,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::{
    categorize::merge_category_batches,
    error::{AppError, Result},
    execute, llm,
    logging::Verbosity,
    models::{
        AppConfig, CategoryTree, KeywordSet, LlmUsageSummary, PlacementDecision,
        PreliminaryCategoryPair, RunReport, SynthesizeCategoriesState,
    },
    place::{
        OutputSnapshot, PlacementBatchProgress, PlacementOptions,
        generate_placements_with_progress, inspect_output,
    },
    planner::build_move_plan,
    report,
    run_state::{ExtractTextState, RunStage, RunWorkspace},
};

use super::planning::{StagePlan, log_resume, log_stage, log_timing};

const PLACEMENT_BATCH_PROGRESS_FILE: &str = "09-generate-placements-partial-batches.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(from = "InspectReviewStateRepr")]
pub(super) struct InspectReviewState {
    pub(super) categories: Vec<CategoryTree>,
    #[serde(skip)]
    pub(super) is_current: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum InspectReviewStateRepr {
    Current {
        categories: Vec<CategoryTree>,
    },
    LegacyReview {
        #[serde(rename = "batch_categories")]
        _batch_categories: Vec<Vec<CategoryTree>>,
        #[serde(default, rename = "last_suggestion")]
        _last_suggestion: Option<String>,
    },
    Legacy {
        #[serde(rename = "is_empty")]
        _is_empty: bool,
        #[serde(rename = "existing_folders")]
        _existing_folders: Vec<String>,
        #[serde(rename = "tree_map")]
        _tree_map: String,
    },
}

impl From<InspectReviewStateRepr> for InspectReviewState {
    fn from(value: InspectReviewStateRepr) -> Self {
        match value {
            InspectReviewStateRepr::Current { categories } => Self {
                categories,
                is_current: true,
            },
            InspectReviewStateRepr::LegacyReview { .. } | InspectReviewStateRepr::Legacy { .. } => {
                Self {
                    categories: Vec::new(),
                    is_current: false,
                }
            }
        }
    }
}

pub(super) async fn inspect_output_stage(
    taxonomy_state: &SynthesizeCategoriesState,
    llm_client: Option<&Arc<dyn llm::LlmClient>>,
    config: &AppConfig,
    report: &mut RunReport,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
) -> Result<Vec<CategoryTree>> {
    let review_client = llm_client.cloned();
    let review_category_depth = config.category_depth;
    let review_subcategories_suggestion_number = config.subcategories_suggestion_number;

    inspect_output_stage_with_interaction(
        taxonomy_state,
        report,
        workspace,
        verbosity,
        stage_plan,
        |categories, prompt_verbosity| {
            prompt_for_inspect_review_action(categories, prompt_verbosity)
        },
        || prompt_for_continue_improving(),
        |_partial_categories, suggestion, current_categories, merge_verbosity| {
            let review_client = review_client.clone();
            let improvement_source = vec![current_categories.to_vec()];
            Box::pin(async move {
                merge_category_batches(
                    review_client
                        .as_ref()
                        .ok_or_else(|| AppError::Execution("missing llm client".to_string()))?
                        .as_ref(),
                    &improvement_source,
                    review_category_depth,
                    review_subcategories_suggestion_number,
                    Some(suggestion),
                    merge_verbosity,
                )
                .await
            })
        },
    )
    .await
}

type ImproveCategoriesFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(Vec<CategoryTree>, LlmUsageSummary)>> + 'a>>;

async fn inspect_output_stage_with_interaction<PA, PC, I>(
    taxonomy_state: &SynthesizeCategoriesState,
    report: &mut RunReport,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
    mut prompt_action: PA,
    mut prompt_continue: PC,
    mut improve_categories: I,
) -> Result<Vec<CategoryTree>>
where
    PA: FnMut(&[CategoryTree], Verbosity) -> Result<InspectReviewAction>,
    PC: FnMut() -> Result<bool>,
    I: for<'a> FnMut(
        &'a [Vec<CategoryTree>],
        &'a str,
        &'a [CategoryTree],
        Verbosity,
    ) -> ImproveCategoriesFuture<'a>,
{
    stage_plan.announce(verbosity, RunStage::InspectOutput);
    if let Some(saved) = workspace.load_stage::<InspectReviewState>(RunStage::InspectOutput)? {
        if saved.is_current {
            log_resume(verbosity, "inspect-output", workspace);
            return Ok(saved.categories);
        }
        verbosity.stage_line(
            "inspect-output",
            "legacy inspect-output state detected; rerendering merged taxonomy".to_string(),
        );
    }

    let stage_started = Instant::now();
    log_stage(
        verbosity,
        "inspect-output",
        format!(
            "reviewing merged taxonomy with {} top-level categor(ies)",
            taxonomy_state.categories.len()
        ),
    );
    let mut categories = taxonomy_state.categories.clone();
    render_inspect_taxonomy(&categories, verbosity);

    let partial_categories = if taxonomy_state.partial_categories.is_empty() {
        vec![categories.clone()]
    } else {
        taxonomy_state.partial_categories.clone()
    };

    loop {
        match prompt_action(&categories, verbosity)? {
            InspectReviewAction::Accept => break,
            InspectReviewAction::Cancel => {
                return Err(AppError::Execution("inspect-output cancelled".to_string()));
            }
            InspectReviewAction::Suggest(suggestion) => {
                let (improved_categories, usage) = improve_categories(
                    &partial_categories,
                    suggestion.as_str(),
                    &categories,
                    verbosity,
                )
                .await?;
                categories = improved_categories;
                report.llm_usage.taxonomy.merge(&usage);
                workspace.save_report(report)?;
                render_inspect_taxonomy(&categories, verbosity);

                if !prompt_continue()? {
                    break;
                }
            }
        }
    }

    workspace.save_stage(
        RunStage::InspectOutput,
        &InspectReviewState {
            categories: categories.clone(),
            is_current: true,
        },
    )?;
    log_timing(verbosity, "inspect-output", stage_started.elapsed());
    Ok(categories)
}

fn render_inspect_taxonomy(categories: &[CategoryTree], verbosity: Verbosity) {
    if !verbosity.quiet() || io::stdin().is_terminal() {
        report::print_category_tree(categories, verbosity);
    }
}

fn prompt_for_inspect_review_action(
    categories: &[CategoryTree],
    verbosity: Verbosity,
) -> Result<InspectReviewAction> {
    if !io::stdin().is_terminal() {
        return Ok(InspectReviewAction::Accept);
    }

    let mut stdin = io::stdin().lock();
    let mut stderr = io::stderr();
    prompt_for_inspect_review_action_with_io(categories, verbosity, &mut stdin, &mut stderr)
}

fn prompt_for_inspect_review_action_with_io<R, W>(
    _categories: &[CategoryTree],
    _verbosity: Verbosity,
    reader: &mut R,
    writer: &mut W,
) -> Result<InspectReviewAction>
where
    R: BufRead,
    W: Write,
{
    let mut input = String::new();
    loop {
        write!(
            writer,
            "Enter a taxonomy improvement suggestion, press Enter to continue, or type 'q' to cancel: "
        )?;
        writer.flush()?;

        input.clear();
        let bytes_read = reader.read_line(&mut input)?;
        if bytes_read == 0 {
            return Err(AppError::Execution(
                "inspect-output cancelled before a review choice was made".to_string(),
            ));
        }

        match resolve_inspect_review_action(input.trim()) {
            Ok(action) => return Ok(action),
            Err(err) => writeln!(writer, "error: {err}")?,
        }
    }
}

fn prompt_for_continue_improving() -> Result<bool> {
    if !io::stdin().is_terminal() {
        return Ok(false);
    }

    let mut stdin = io::stdin().lock();
    let mut stderr = io::stderr();
    prompt_for_continue_improving_with_io(&mut stdin, &mut stderr)
}

fn prompt_for_continue_improving_with_io<R, W>(reader: &mut R, writer: &mut W) -> Result<bool>
where
    R: BufRead,
    W: Write,
{
    let mut input = String::new();
    loop {
        write!(
            writer,
            "Continue improving this taxonomy? [y/N] (or 'q' to cancel): "
        )?;
        writer.flush()?;

        input.clear();
        let bytes_read = reader.read_line(&mut input)?;
        if bytes_read == 0 {
            return Err(AppError::Execution(
                "inspect-output cancelled before a continuation choice was made".to_string(),
            ));
        }

        match resolve_continue_improving(input.trim()) {
            Ok(InspectLoopDecision::ContinueImproving) => return Ok(true),
            Ok(InspectLoopDecision::Finish) => return Ok(false),
            Ok(InspectLoopDecision::Cancel) => {
                return Err(AppError::Execution("inspect-output cancelled".to_string()));
            }
            Err(err) => writeln!(writer, "error: {err}")?,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InspectReviewAction {
    Accept,
    Cancel,
    Suggest(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InspectLoopDecision {
    ContinueImproving,
    Finish,
    Cancel,
}

fn resolve_inspect_review_action(input: &str) -> Result<InspectReviewAction> {
    if input.is_empty() {
        return Ok(InspectReviewAction::Accept);
    }

    if input.eq_ignore_ascii_case("q") || input.eq_ignore_ascii_case("quit") {
        return Ok(InspectReviewAction::Cancel);
    }

    Ok(InspectReviewAction::Suggest(input.to_string()))
}

fn resolve_continue_improving(input: &str) -> Result<InspectLoopDecision> {
    if input.is_empty() || input.eq_ignore_ascii_case("n") || input.eq_ignore_ascii_case("no") {
        return Ok(InspectLoopDecision::Finish);
    }

    if input.eq_ignore_ascii_case("y") || input.eq_ignore_ascii_case("yes") {
        return Ok(InspectLoopDecision::ContinueImproving);
    }

    if input.eq_ignore_ascii_case("q") || input.eq_ignore_ascii_case("quit") {
        return Ok(InspectLoopDecision::Cancel);
    }

    Err(AppError::Execution(
        "enter 'y' to keep improving, press Enter to continue, or type 'q' to cancel".to_string(),
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn generate_placements_stage(
    saved_placements: Option<Vec<PlacementDecision>>,
    llm_client: Option<&Arc<dyn llm::LlmClient>>,
    extract_state: &ExtractTextState,
    keyword_sets: &[KeywordSet],
    preliminary_pairs: &[PreliminaryCategoryPair],
    categories: &[CategoryTree],
    config: &AppConfig,
    report: &mut RunReport,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
) -> Result<Vec<PlacementDecision>> {
    stage_plan.announce(verbosity, RunStage::GeneratePlacements);
    if let Some(saved) = saved_placements {
        workspace.remove_artifact(PLACEMENT_BATCH_PROGRESS_FILE)?;
        log_resume(verbosity, "generate-placements", workspace);
        return Ok(saved);
    }

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
    let live_snapshot = inspect_output(Path::new(&config.output))?;
    let placement_snapshot = pick_snapshot_for_mode(&live_snapshot, config.rebuild);
    let saved_progress = workspace
        .load_artifact::<PlacementBatchProgress>(PLACEMENT_BATCH_PROGRESS_FILE)?
        .unwrap_or_default();
    if !saved_progress.completed_batches.is_empty() {
        report.llm_usage.placements = saved_progress.usage.clone();
        workspace.save_report(report)?;
    }
    let (placements, usage) = generate_placements_with_progress(
        Arc::clone(require_llm_client(llm_client)?),
        &extract_state.papers,
        keyword_sets,
        preliminary_pairs,
        categories,
        &placement_snapshot,
        PlacementOptions {
            batch_size: config.placement_batch_size,
            batch_start_delay_ms: config.batch_start_delay_ms,
            placement_mode: config.placement_mode,
            category_depth: config.category_depth,
            verbosity,
        },
        saved_progress,
        |progress| {
            report.llm_usage.placements = progress.usage.clone();
            workspace.save_artifact(PLACEMENT_BATCH_PROGRESS_FILE, progress)?;
            workspace.save_report(report)
        },
    )
    .await?;
    report.llm_usage.placements = usage;
    workspace.save_stage(RunStage::GeneratePlacements, &placements)?;
    workspace.remove_artifact(PLACEMENT_BATCH_PROGRESS_FILE)?;
    workspace.save_report(report)?;
    log_stage(
        verbosity,
        "generate-placements",
        format!("generated {} placement decision(s)", placements.len()),
    );
    log_timing(verbosity, "generate-placements", stage_started.elapsed());
    Ok(placements)
}

pub(super) fn build_plan_stage(
    extract_state: &ExtractTextState,
    placements: &[PlacementDecision],
    config: &AppConfig,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
) -> Result<Vec<crate::models::PlanAction>> {
    stage_plan.announce(verbosity, RunStage::BuildPlan);
    if let Some(saved) = workspace.load_stage::<Vec<_>>(RunStage::BuildPlan)? {
        log_resume(verbosity, "build-plan", workspace);
        return Ok(saved);
    }

    let stage_started = Instant::now();
    log_stage(
        verbosity,
        "build-plan",
        format!("building move plan rooted at {}", config.output.display()),
    );
    let actions = build_move_plan(Path::new(&config.output), &extract_state.papers, placements)?;
    workspace.save_stage(RunStage::BuildPlan, &actions)?;
    log_stage(
        verbosity,
        "build-plan",
        format!("planned {} filesystem action(s)", actions.len()),
    );
    log_timing(verbosity, "build-plan", stage_started.elapsed());
    Ok(actions)
}

pub(super) fn execute_plan_stage(
    report: &RunReport,
    config: &AppConfig,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
) -> Result<()> {
    stage_plan.announce(verbosity, RunStage::ExecutePlan);
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
    execute::execute_plan(&report.actions, config.dry_run, verbosity)?;
    workspace.mark_stage(RunStage::ExecutePlan)?;
    log_stage(verbosity, "execute-plan", "execution complete".to_string());
    log_timing(verbosity, "execute-plan", stage_started.elapsed());
    Ok(())
}

pub(super) fn finalize_empty_run(
    report: RunReport,
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    elapsed: Duration,
) -> Result<RunReport> {
    if !verbosity.quiet() {
        report::print_report(&report, verbosity);
    }
    workspace.save_report(&report)?;
    workspace.mark_completed()?;
    if report.failed > 0 {
        return Err(AppError::Execution(
            "run completed with extraction failures and no processable papers".to_string(),
        ));
    }
    log_timing(verbosity, "total", elapsed);
    Ok(report)
}

pub(super) fn pick_snapshot_for_mode(snapshot: &OutputSnapshot, rebuild: bool) -> OutputSnapshot {
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

fn require_llm_client(
    client: Option<&Arc<dyn llm::LlmClient>>,
) -> Result<&Arc<dyn llm::LlmClient>> {
    client.ok_or_else(|| AppError::Execution("missing llm client".to_string()))
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, path::PathBuf};

    use tempfile::tempdir;

    use super::{
        InspectLoopDecision, InspectReviewAction, InspectReviewState,
        inspect_output_stage_with_interaction, prompt_for_continue_improving_with_io,
        resolve_continue_improving, resolve_inspect_review_action,
    };
    use crate::{
        app_run::stages::planning::StagePlan,
        error::AppError,
        logging::Verbosity,
        models::{
            AppConfig, CategoryTree, LlmProvider, PlacementMode, RunReport,
            SynthesizeCategoriesState, TaxonomyMode,
        },
        run_state::{RunStage, RunWorkspace},
    };

    #[test]
    fn inspect_review_action_accepts_blank_input() {
        assert_eq!(
            resolve_inspect_review_action("").expect("blank input should continue"),
            InspectReviewAction::Accept
        );
    }

    #[test]
    fn inspect_review_action_treats_text_as_suggestion() {
        assert_eq!(
            resolve_inspect_review_action("merge speech categories")
                .expect("suggestion should parse"),
            InspectReviewAction::Suggest("merge speech categories".to_string())
        );
    }

    #[test]
    fn inspect_review_action_allows_cancel() {
        assert_eq!(
            resolve_inspect_review_action("q").expect("cancel should parse"),
            InspectReviewAction::Cancel
        );
    }

    #[test]
    fn continue_improving_prompt_rejects_invalid_input_until_valid_line() {
        let mut input = b"maybe\ny\n".as_slice();
        let mut output = Vec::new();

        let keep_improving =
            prompt_for_continue_improving_with_io(&mut input, &mut output).expect("prompt");

        assert!(keep_improving);
        let rendered = String::from_utf8(output).expect("utf8");
        assert!(rendered.contains(
            "error: enter 'y' to keep improving, press Enter to continue, or type 'q' to cancel"
        ));
    }

    #[test]
    fn continue_improving_defaults_to_finish_on_blank_input() {
        assert_eq!(
            resolve_continue_improving("").expect("blank should finish"),
            InspectLoopDecision::Finish
        );
    }

    #[tokio::test]
    async fn inspect_stage_saves_state_only_after_acceptance() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let config = sample_config(dir.path().join("sorted"));
        let mut workspace =
            RunWorkspace::create_with_cache_root_for_tests(dir.path(), &cache_root, &config)
                .expect("create workspace");
        let stage_plan = StagePlan::new(&config, true);
        let mut report = RunReport::new(true);
        let taxonomy_state = sample_taxonomy_state();

        let err = inspect_output_stage_with_interaction(
            &taxonomy_state,
            &mut report,
            &mut workspace,
            Verbosity::new(false, false, false),
            &stage_plan,
            |_categories, _verbosity| {
                Err(AppError::Execution("inspect-output cancelled".to_string()))
            },
            || Ok(false),
            |_partials, _suggestion, current_categories, _verbosity| {
                let current_categories = current_categories.to_vec();
                Box::pin(async move { Ok((current_categories, Default::default())) })
            },
        )
        .await
        .expect_err("stage should stop on cancellation");

        assert!(err.to_string().contains("inspect-output cancelled"));
        assert!(
            workspace
                .load_stage::<InspectReviewState>(RunStage::InspectOutput)
                .expect("load stage")
                .is_none()
        );
    }

    #[tokio::test]
    async fn inspect_stage_applies_suggestions_until_user_finishes() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let config = sample_config(dir.path().join("sorted"));
        let mut workspace =
            RunWorkspace::create_with_cache_root_for_tests(dir.path(), &cache_root, &config)
                .expect("create workspace");
        let stage_plan = StagePlan::new(&config, true);
        let mut report = RunReport::new(true);
        let taxonomy_state = sample_taxonomy_state();
        let prompt_calls = Cell::new(0_u8);
        let continue_calls = Cell::new(0_u8);

        let categories = inspect_output_stage_with_interaction(
            &taxonomy_state,
            &mut report,
            &mut workspace,
            Verbosity::new(false, false, false),
            &stage_plan,
            |_categories, _verbosity| match prompt_calls.get() {
                0 => {
                    prompt_calls.set(1);
                    Ok(InspectReviewAction::Suggest(
                        "Merge speech categories".to_string(),
                    ))
                }
                _ => Ok(InspectReviewAction::Accept),
            },
            || match continue_calls.get() {
                0 => {
                    continue_calls.set(1);
                    Ok(true)
                }
                _ => Ok(false),
            },
            |_partials, suggestion, current_categories, _verbosity| {
                let mut improved = current_categories.to_vec();
                improved[0].name = format!("{} ({suggestion})", improved[0].name);
                Box::pin(async move { Ok((improved, sample_usage())) })
            },
        )
        .await
        .expect("inspect review");

        assert_eq!(categories[0].name, "AI (Merge speech categories)");
        assert_eq!(report.llm_usage.taxonomy.call_count, 1);
        assert_eq!(prompt_calls.get(), 1);
        assert_eq!(continue_calls.get(), 1);
        assert_eq!(
            workspace
                .load_stage::<InspectReviewState>(RunStage::InspectOutput)
                .expect("load stage")
                .expect("saved state")
                .categories[0]
                .name,
            "AI (Merge speech categories)"
        );
    }

    #[tokio::test]
    async fn inspect_stage_skips_review_when_current_state_exists() {
        let dir = tempdir().expect("tempdir");
        let cache_root = dir.path().join("cache");
        let config = sample_config(dir.path().join("sorted"));
        let mut workspace =
            RunWorkspace::create_with_cache_root_for_tests(dir.path(), &cache_root, &config)
                .expect("create workspace");
        let stage_plan = StagePlan::new(&config, true);
        let mut report = RunReport::new(true);
        let taxonomy_state = sample_taxonomy_state();
        let prompt_calls = Cell::new(0_u8);

        inspect_output_stage_with_interaction(
            &taxonomy_state,
            &mut report,
            &mut workspace,
            Verbosity::new(false, false, false),
            &stage_plan,
            |_categories, _verbosity| Ok(InspectReviewAction::Accept),
            || Ok(false),
            |_partials, _suggestion, current_categories, _verbosity| {
                let current_categories = current_categories.to_vec();
                Box::pin(async move { Ok((current_categories, Default::default())) })
            },
        )
        .await
        .expect("initial inspect");

        let categories = inspect_output_stage_with_interaction(
            &taxonomy_state,
            &mut report,
            &mut workspace,
            Verbosity::new(false, false, false),
            &stage_plan,
            |_categories, _verbosity| {
                prompt_calls.set(prompt_calls.get() + 1);
                Ok(InspectReviewAction::Accept)
            },
            || Ok(false),
            |_partials, _suggestion, current_categories, _verbosity| {
                let current_categories = current_categories.to_vec();
                Box::pin(async move { Ok((current_categories, Default::default())) })
            },
        )
        .await
        .expect("resume inspect");

        assert_eq!(prompt_calls.get(), 0);
        assert_eq!(categories[0].name, "AI");
    }

    fn sample_config(output: PathBuf) -> AppConfig {
        AppConfig {
            input: PathBuf::from("/tmp/in"),
            output,
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
            subcategories_suggestion_number: 5,
            verbose: false,
            debug: false,
            quiet: false,
        }
    }

    fn sample_taxonomy_state() -> SynthesizeCategoriesState {
        let categories = vec![CategoryTree {
            name: "AI".to_string(),
            children: vec![CategoryTree {
                name: "Vision".to_string(),
                children: Vec::new(),
            }],
        }];
        SynthesizeCategoriesState {
            categories: categories.clone(),
            partial_categories: vec![categories],
        }
    }

    fn sample_usage() -> crate::models::LlmUsageSummary {
        let mut usage = crate::models::LlmUsageSummary::default();
        usage.call_count = 1;
        usage
    }
}
