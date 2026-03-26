use std::{path::Path, time::Duration};

use serde::Serialize;

use crate::{
    config::AppConfig,
    error::{AppError, Result},
    papers::{
        KeywordSet, KeywordStageState, PaperText, PdfCandidate, PreliminaryCategoryPair,
        SynthesizeCategoriesState, placement::PlacementDecision, taxonomy::CategoryTree,
    },
    report::{FileAction, PlanAction, RunReport},
    session::{ExtractTextState, FilterSizeState, RunStage, RunWorkspace},
    terminal::{self, InspectReviewPrompt, InspectReviewRequest, Verbosity},
};

use super::path_resolution::absolutize_config;

const DEBUG_TUI_PROGRESS_DELAY: Duration = Duration::from_millis(200);
const DEBUG_TUI_PROGRESS_SETTLE_DELAY: Duration = Duration::from_millis(250);

pub async fn run_debug_tui(config: AppConfig) -> Result<RunReport> {
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

pub(crate) fn seed_debug_stages(
    workspace: &mut RunWorkspace,
    config: &AppConfig,
) -> Result<DebugRunData> {
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
            reference_evidence: None,
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

pub(crate) struct DebugRunData {
    pub(crate) categories: Vec<CategoryTree>,
    pub(crate) report: RunReport,
}

pub(crate) fn simulate_debug_taxonomy_review(
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

pub(crate) fn apply_debug_taxonomy_suggestion(
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
