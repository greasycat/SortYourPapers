use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::{
    error::{AppError, Result},
    execute, llm,
    logging::Verbosity,
    models::{
        AppConfig, CategoryTree, KeywordSet, PlacementDecision, PreliminaryCategoryPair, RunReport,
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

pub(super) fn inspect_output_stage(
    categories: &[CategoryTree],
    workspace: &mut RunWorkspace,
    verbosity: Verbosity,
    stage_plan: &StagePlan,
) -> Result<()> {
    stage_plan.announce(verbosity, RunStage::InspectOutput);
    if let Some(saved) = workspace.load_stage::<InspectReviewState>(RunStage::InspectOutput)? {
        if saved.is_current {
            log_resume(verbosity, "inspect-output", workspace);
            return Ok(());
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
            categories.len()
        ),
    );
    if !verbosity.quiet() {
        report::print_category_tree(categories, verbosity);
    }
    workspace.save_stage(
        RunStage::InspectOutput,
        &InspectReviewState {
            categories: categories.to_vec(),
            is_current: true,
        },
    )?;
    log_timing(verbosity, "inspect-output", stage_started.elapsed());
    Ok(())
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
