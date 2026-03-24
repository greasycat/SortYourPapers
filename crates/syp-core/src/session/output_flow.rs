use std::{
    future::Future,
    path::Path,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    config::AppConfig,
    error::{AppError, Result},
    llm,
    llm::LlmUsageSummary,
    papers::fs_ops::{execute::execute_plan, planner::build_move_plan},
    papers::placement::PlacementDecision,
    papers::placement::{
        OutputSnapshot, PlacementBatchProgress, PlacementOptions,
        generate_placements_with_progress, inspect_output,
    },
    papers::taxonomy::{CategoryTree, merge_category_batches},
    papers::{KeywordSet, PreliminaryCategoryPair, SynthesizeCategoriesState},
    report::{PlanAction, RunReport},
    session::{ExtractTextState, RunStage, RunWorkspace},
    terminal::{self, InspectReviewPrompt, Verbosity},
};

use super::runtime::{StagePlan, log_resume, log_stage, log_timing};

const PLACEMENT_BATCH_PROGRESS_FILE: &str = "09-generate-placements-partial-batches.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct InspectReviewState {
    pub(super) categories: Vec<CategoryTree>,
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
    let review_existing_output_folders = existing_output_folders_for_taxonomy_merge(config)?;
    let review_reference_evidence = taxonomy_state.reference_evidence.clone();

    inspect_output_stage_with_interaction(
        taxonomy_state,
        report,
        workspace,
        verbosity,
        stage_plan,
        |categories, prompt_verbosity| {
            terminal::prompt_inspect_review_action(categories, prompt_verbosity)
        },
        || terminal::prompt_continue_improving(),
        |_partial_categories, suggestion, current_categories, merge_verbosity| {
            let review_client = review_client.clone();
            let improvement_source = vec![current_categories.to_vec()];
            let review_existing_output_folders = review_existing_output_folders.clone();
            let review_reference_evidence = review_reference_evidence.clone();
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
                    review_existing_output_folders.as_deref(),
                    review_reference_evidence.as_ref(),
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
    PA: FnMut(&[CategoryTree], Verbosity) -> Result<InspectReviewPrompt>,
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
        log_resume(verbosity, "inspect-output", workspace);
        return Ok(saved.categories);
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
            InspectReviewPrompt::Accept => break,
            InspectReviewPrompt::Cancel => {
                return Err(AppError::Execution("inspect-output cancelled".to_string()));
            }
            InspectReviewPrompt::Suggest(request) => {
                let llm_request = request.llm_request();
                let (improved_categories, usage) = improve_categories(
                    &partial_categories,
                    llm_request.as_str(),
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
        },
    )?;
    log_timing(verbosity, "inspect-output", stage_started.elapsed());
    Ok(categories)
}

fn render_inspect_taxonomy(categories: &[CategoryTree], verbosity: Verbosity) {
    if !verbosity.quiet() || terminal::terminal_is_interactive() {
        terminal::report::print_category_tree(categories, verbosity);
    }
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
) -> Result<Vec<PlanAction>> {
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
    execute_plan(&report.actions, config.dry_run, verbosity)?;
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
        terminal::report::print_report(&report, verbosity);
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

pub(super) fn existing_output_folders_for_taxonomy_merge(
    config: &AppConfig,
) -> Result<Option<Vec<String>>> {
    if config.rebuild || !config.use_current_folder_tree {
        return Ok(None);
    }

    let snapshot = inspect_output(Path::new(&config.output))?;
    if snapshot.is_empty {
        return Ok(None);
    }

    let folders = snapshot
        .existing_folders
        .into_iter()
        .filter(|folder| folder != ".")
        .collect::<Vec<_>>();
    if folders.is_empty() {
        Ok(None)
    } else {
        Ok(Some(folders))
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
        InspectReviewState, existing_output_folders_for_taxonomy_merge,
        inspect_output_stage_with_interaction,
    };
    use crate::{
        config::AppConfig,
        error::AppError,
        llm::{LlmProvider, LlmUsageSummary},
        papers::SynthesizeCategoriesState,
        papers::placement::PlacementMode,
        papers::taxonomy::{CategoryTree, TaxonomyAssistance, TaxonomyMode},
        report::RunReport,
        session::{RunStage, RunWorkspace, runtime::StagePlan},
        terminal::InspectReviewPrompt,
        terminal::Verbosity,
    };

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
                    Ok(InspectReviewPrompt::Suggest(
                        crate::terminal::InspectReviewRequest::from_user_suggestion(
                            "Merge speech categories".to_string(),
                        ),
                    ))
                }
                _ => Ok(InspectReviewPrompt::Accept),
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
            |_categories, _verbosity| Ok(InspectReviewPrompt::Accept),
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
                Ok(InspectReviewPrompt::Accept)
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

    #[test]
    fn current_folder_tree_hint_uses_explicit_flag_not_placement_mode() {
        let dir = tempdir().expect("tempdir");
        let output = dir.path().join("sorted");
        std::fs::create_dir_all(output.join("AI/Vision")).expect("create folders");
        let mut config = sample_config(output);
        config.use_current_folder_tree = true;
        config.placement_mode = PlacementMode::ExistingOnly;

        let folders = existing_output_folders_for_taxonomy_merge(&config)
            .expect("collect folders")
            .expect("folders");

        assert!(folders.contains(&"AI".to_string()));
        assert!(folders.contains(&"AI/Vision".to_string()));
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
            taxonomy_assistance: TaxonomyAssistance::LlmOnly,
            taxonomy_batch_size: 4,
            reference_manifest_path: PathBuf::from("assets/testsets/scijudgebench-diverse.toml"),
            reference_top_k: 5,
            use_current_folder_tree: false,
            placement_batch_size: 10,
            placement_mode: PlacementMode::ExistingOnly,
            rebuild: false,
            dry_run: true,
            llm_provider: LlmProvider::Gemini,
            llm_model: "gemini-3-flash-preview".to_string(),
            llm_base_url: None,
            api_key: None,
            embedding_provider: LlmProvider::Gemini,
            embedding_model: "text-embedding-004".to_string(),
            embedding_base_url: None,
            embedding_api_key: None,
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
            reference_evidence: None,
        }
    }

    fn sample_usage() -> LlmUsageSummary {
        let mut usage = LlmUsageSummary::default();
        usage.call_count = 1;
        usage
    }
}
