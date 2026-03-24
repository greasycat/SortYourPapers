use std::{collections::HashMap, sync::Arc, time::Instant};

use serde::Deserialize;
use serde_json::Value;

use crate::{
    error::{AppError, Result},
    llm::LlmUsageSummary,
    llm::{JsonResponseSchema, LlmClient, call_json_with_retry},
    papers::taxonomy::CategoryTree,
    papers::{KeywordSet, PaperText, PreliminaryCategoryPair},
    terminal::{ProgressTracker, format_duration},
};

use super::{
    MAX_JSON_ATTEMPTS, MAX_SEMANTIC_ATTEMPTS, OutputSnapshot, PlacementBatchProgress,
    PlacementBatchResult, PlacementBatchRuntime, PlacementDecision, PlacementOptions,
    batching::{batch_dispatch_spacing, wait_for_dispatch_slot},
    prompts::{
        build_allowed_targets, build_file_context, build_placement_prompt, format_paper_batch_span,
        format_placement_request_debug_message,
    },
    validation::validate_placements,
};

#[derive(Debug, Deserialize)]
struct PlacementResponse {
    placements: Vec<PlacementDecision>,
}

#[derive(Debug, Clone)]
struct PreparedPlacementBatch {
    batch_index: usize,
    papers: Vec<PaperText>,
    file_context: Vec<Value>,
}

/// Generates placement decisions for a set of papers.
///
/// # Errors
/// Returns an error when batching is misconfigured, an LLM request fails, or
/// the resulting placement decisions fail validation.
pub async fn generate_placements(
    client: Arc<dyn LlmClient>,
    papers: &[PaperText],
    keyword_sets: &[KeywordSet],
    preliminary_pairs: &[PreliminaryCategoryPair],
    categories: &[CategoryTree],
    snapshot: &OutputSnapshot,
    options: PlacementOptions,
) -> Result<(Vec<PlacementDecision>, LlmUsageSummary)> {
    generate_placements_with_progress(
        client,
        papers,
        keyword_sets,
        preliminary_pairs,
        categories,
        snapshot,
        options,
        PlacementBatchProgress::default(),
        |_| Ok(()),
    )
    .await
}

pub(crate) async fn generate_placements_with_progress<F>(
    client: Arc<dyn LlmClient>,
    papers: &[PaperText],
    keyword_sets: &[KeywordSet],
    preliminary_pairs: &[PreliminaryCategoryPair],
    categories: &[CategoryTree],
    snapshot: &OutputSnapshot,
    options: PlacementOptions,
    saved_progress: PlacementBatchProgress,
    mut on_progress: F,
) -> Result<(Vec<PlacementDecision>, LlmUsageSummary)>
where
    F: FnMut(&PlacementBatchProgress) -> Result<()>,
{
    if papers.is_empty() {
        return Ok((Vec::new(), LlmUsageSummary::default()));
    }
    if options.batch_size == 0 {
        return Err(AppError::Validation(
            "placement_batch_size must be greater than 0".to_string(),
        ));
    }

    let keyword_map: HashMap<&str, &[String]> = keyword_sets
        .iter()
        .map(|set| (set.file_id.as_str(), set.keywords.as_slice()))
        .collect();
    let preliminary_map: HashMap<&str, &str> = preliminary_pairs
        .iter()
        .map(|pair| {
            (
                pair.file_id.as_str(),
                pair.preliminary_categories_k_depth.as_str(),
            )
        })
        .collect();
    let mut ordered_papers = papers.to_vec();
    ordered_papers.sort_by(|left, right| {
        left.file_id
            .cmp(&right.file_id)
            .then_with(|| left.path.cmp(&right.path))
    });
    let total_batches = ordered_papers.len().div_ceil(options.batch_size);
    options.verbosity.stage_line(
        "placements",
        format!(
            "{} paper(s) ready; batching into {} request(s) of up to {} file(s)",
            ordered_papers.len(),
            total_batches,
            options.batch_size
        ),
    );

    let prepared_batches = ordered_papers
        .chunks(options.batch_size)
        .enumerate()
        .map(|(batch_index, batch)| PreparedPlacementBatch {
            batch_index: batch_index + 1,
            papers: batch.to_vec(),
            file_context: build_file_context(batch, &keyword_map, &preliminary_map),
        })
        .collect::<Vec<_>>();
    let mut progress_state = validate_saved_placement_progress(
        &prepared_batches,
        snapshot,
        options.placement_mode,
        options.category_depth,
        saved_progress,
    )?;
    let resumed_batches = progress_state.completed_batches.len();
    if resumed_batches > 0 {
        options.verbosity.stage_line(
            "placements",
            format!("resuming {} saved placement batch(es)", resumed_batches),
        );
    }
    let runtime = PlacementBatchRuntime {
        categories: Arc::new(categories.to_vec()),
        snapshot: Arc::new(snapshot.clone()),
        options,
        total_batches,
    };
    let completed_indexes = progress_state
        .completed_batches
        .iter()
        .map(|batch| batch.batch_index)
        .collect::<std::collections::HashSet<_>>();
    let mut usage = progress_state.usage.clone();
    let mut next_dispatch_at = None;
    let dispatch_spacing = batch_dispatch_spacing(runtime.options.batch_start_delay_ms);
    let mut progress = ProgressTracker::new(
        runtime.options.verbosity,
        total_batches,
        "placement batches",
        true,
    );
    progress.inc(progress_state.completed_batches.len());

    for batch in prepared_batches
        .iter()
        .filter(|batch| !completed_indexes.contains(&batch.batch_index))
    {
        wait_for_dispatch_slot(&mut next_dispatch_at, dispatch_spacing).await;
        let started_at = Instant::now();
        let batch_span = format_paper_batch_span(&batch.papers);
        runtime.options.verbosity.stage_line(
            "placements",
            format!(
                "batch {}/{} {}",
                batch.batch_index, runtime.total_batches, batch_span
            ),
        );
        let (placements, batch_usage) =
            match generate_placement_batch(client.as_ref(), batch, &runtime).await {
                Ok(result) => result,
                Err(err) => {
                    runtime.options.verbosity.error_line(
                        "PLACEMENTS",
                        format!(
                            "batch {}/{} failed after {} {}: {}",
                            batch.batch_index,
                            runtime.total_batches,
                            format_duration(started_at.elapsed()),
                            batch_span,
                            err
                        ),
                    );
                    return Err(err);
                }
            };
        let elapsed = started_at.elapsed();
        runtime.options.verbosity.success_line(
            "PLACEMENTS",
            format!(
                "batch {}/{} completed in {} {}",
                batch.batch_index,
                runtime.total_batches,
                format_duration(elapsed),
                batch_span
            ),
        );
        usage.merge(&batch_usage);
        progress_state.completed_batches.push(PlacementBatchResult {
            batch_index: batch.batch_index,
            file_ids: batch
                .papers
                .iter()
                .map(|paper| paper.file_id.clone())
                .collect(),
            placements,
            elapsed_ms: elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
        });
        progress_state
            .completed_batches
            .sort_by_key(|saved_batch| saved_batch.batch_index);
        progress_state.usage = usage.clone();
        on_progress(&progress_state)?;
        progress.inc(1);
    }
    progress.finish();

    let mut all_placements = Vec::with_capacity(papers.len());
    for batch in &progress_state.completed_batches {
        all_placements.extend(batch.placements.clone());
    }
    validate_placements(
        &all_placements,
        papers,
        snapshot,
        options.placement_mode,
        options.category_depth,
    )?;
    options.verbosity.success_line(
        "PLACEMENTS",
        format!(
            "completed {} placement batch(es) and collected {} decision(s)",
            total_batches,
            all_placements.len()
        ),
    );

    Ok((all_placements, usage))
}

async fn generate_placement_batch(
    client: &dyn LlmClient,
    batch: &PreparedPlacementBatch,
    runtime: &PlacementBatchRuntime,
) -> Result<(Vec<PlacementDecision>, LlmUsageSummary)> {
    let system = "You assign PDFs to category folders. Return strict JSON only.";
    let allowed_targets = build_allowed_targets(
        runtime.categories.as_slice(),
        runtime.snapshot.as_ref(),
        runtime.options.placement_mode,
        runtime.options.category_depth,
    );
    let base_user = build_placement_prompt(&batch.file_context, &allowed_targets)?;
    let mut user = base_user.clone();
    let mut usage = LlmUsageSummary::default();
    let mut last_issue = String::new();

    for attempt in 1..=MAX_SEMANTIC_ATTEMPTS {
        if runtime.options.verbosity.debug_enabled() {
            runtime.options.verbosity.debug_line(
                "LLM",
                format!(
                    "placement batch {}/{}\n{}",
                    batch.batch_index,
                    runtime.total_batches,
                    format_placement_request_debug_message(system, &user)
                ),
            );
        }

        let schema = placement_response_schema();
        let mut response: crate::llm::ParsedLlmResponse<PlacementResponse> =
            call_json_with_retry(client, system, &user, &schema, MAX_JSON_ATTEMPTS).await?;
        match validate_placements(
            &response.value.placements,
            &batch.papers,
            runtime.snapshot.as_ref(),
            runtime.options.placement_mode,
            runtime.options.category_depth,
        ) {
            Ok(()) => {
                usage.record_call(&response.metrics);
                return Ok((response.value.placements, usage));
            }
            Err(err) => last_issue = err.to_string(),
        }

        if attempt < MAX_SEMANTIC_ATTEMPTS {
            response.metrics.semantic_retry_count += 1;
        }
        usage.record_call(&response.metrics);

        if attempt < MAX_SEMANTIC_ATTEMPTS {
            runtime.options.verbosity.warn_line(
                "PLACEMENTS",
                format!(
                    "batch {}/{} retry {}/{}: {}",
                    batch.batch_index,
                    runtime.total_batches,
                    attempt + 1,
                    MAX_SEMANTIC_ATTEMPTS,
                    last_issue
                ),
            );
            user = format!(
                "{base_user}\n\nYour previous response had this issue: {last_issue}.\nReturn JSON again.\nImportant: return exactly one placement for every file_id in this batch only."
            );
        }
    }

    Err(AppError::Validation(format!(
        "failed placement validation for batch {}/{}: {}",
        batch.batch_index, runtime.total_batches, last_issue
    )))
}

fn validate_saved_placement_progress(
    prepared_batches: &[PreparedPlacementBatch],
    snapshot: &OutputSnapshot,
    placement_mode: crate::papers::placement::PlacementMode,
    category_depth: u8,
    mut progress: PlacementBatchProgress,
) -> Result<PlacementBatchProgress> {
    if progress.completed_batches.is_empty() {
        return Ok(progress);
    }

    let expected_batches = prepared_batches
        .iter()
        .map(|batch| {
            (
                batch.batch_index,
                batch
                    .papers
                    .iter()
                    .map(|paper| paper.file_id.clone())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<HashMap<_, _>>();

    for batch in &progress.completed_batches {
        let Some(expected_file_ids) = expected_batches.get(&batch.batch_index) else {
            return Err(AppError::Validation(format!(
                "saved placement batch {} no longer matches the current input",
                batch.batch_index
            )));
        };
        if &batch.file_ids != expected_file_ids {
            return Err(AppError::Validation(format!(
                "saved placement batch {} has inconsistent file ids",
                batch.batch_index
            )));
        }
        let batch_papers = prepared_batches
            .iter()
            .find(|prepared| prepared.batch_index == batch.batch_index)
            .map(|prepared| prepared.papers.as_slice())
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "saved placement batch {} no longer matches the current input",
                    batch.batch_index
                ))
            })?;
        validate_placements(
            &batch.placements,
            batch_papers,
            snapshot,
            placement_mode,
            category_depth,
        )?;
    }

    progress
        .completed_batches
        .sort_by_key(|batch| batch.batch_index);
    Ok(progress)
}

fn placement_response_schema() -> JsonResponseSchema {
    JsonResponseSchema::new(
        "placement_response",
        serde_json::json!({
            "type": "object",
            "properties": {
                "placements": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "file_id": {
                                "type": "string"
                            },
                            "target_rel_path": {
                                "type": "string"
                            }
                        },
                        "required": ["file_id", "target_rel_path"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["placements"],
            "additionalProperties": false
        }),
    )
}
