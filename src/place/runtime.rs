use std::{collections::HashMap, sync::Arc, time::Instant};

use serde::Deserialize;
use serde_json::Value;
use tokio::task::JoinSet;

use crate::{
    error::{AppError, Result},
    llm::{JsonResponseSchema, LlmClient, call_json_with_retry},
    logging::{ProgressTracker, Verbosity, format_duration},
    models::{
        CategoryTree, KeywordSet, LlmUsageSummary, PaperText, PlacementDecision,
        PreliminaryCategoryPair,
    },
};

use super::{
    MAX_CONCURRENT_PLACEMENT_BATCH_REQUESTS, MAX_JSON_ATTEMPTS, MAX_SEMANTIC_ATTEMPTS,
    OutputSnapshot, PlacementBatchRuntime, PlacementOptions,
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
    let total_batches = papers.len().div_ceil(options.batch_size);
    options.verbosity.stage_line(
        "placements",
        format!(
            "{} paper(s) ready; batching into {} request(s) of up to {} file(s)",
            papers.len(),
            total_batches,
            options.batch_size
        ),
    );

    let prepared_batches = papers
        .chunks(options.batch_size)
        .enumerate()
        .map(|(batch_index, batch)| PreparedPlacementBatch {
            batch_index: batch_index + 1,
            papers: batch.to_vec(),
            file_context: build_file_context(batch, &keyword_map, &preliminary_map),
        })
        .collect::<Vec<_>>();
    let runtime = PlacementBatchRuntime {
        categories: Arc::new(categories.to_vec()),
        snapshot: Arc::new(snapshot.clone()),
        options: PlacementOptions {
            verbosity: if options.verbosity.show_progress(total_batches, true) {
                options.verbosity.stage_silenced()
            } else {
                options.verbosity
            },
            ..options
        },
        total_batches,
    };
    let (batch_results, usage) = run_placement_batches_concurrently(
        client,
        prepared_batches,
        runtime.clone(),
        options.verbosity,
    )
    .await?;
    let mut all_placements = Vec::with_capacity(papers.len());
    for (_, placements) in batch_results {
        all_placements.extend(placements);
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

async fn run_placement_batches_concurrently(
    client: Arc<dyn LlmClient>,
    prepared_batches: Vec<PreparedPlacementBatch>,
    runtime: PlacementBatchRuntime,
    verbosity: Verbosity,
) -> Result<(Vec<(usize, Vec<PlacementDecision>)>, LlmUsageSummary)> {
    let max_in_flight = MAX_CONCURRENT_PLACEMENT_BATCH_REQUESTS.max(1);
    let dispatch_spacing = batch_dispatch_spacing(runtime.options.batch_start_delay_ms);
    let mut pending_batches = prepared_batches.into_iter();
    let mut in_flight = JoinSet::new();
    let mut batch_results = Vec::with_capacity(runtime.total_batches);
    let mut usage = LlmUsageSummary::default();
    let mut next_dispatch_at = None;
    let mut progress =
        ProgressTracker::new(verbosity, runtime.total_batches, "placement batches", true);

    for _ in 0..max_in_flight {
        let Some(batch) = pending_batches.next() else {
            break;
        };
        wait_for_dispatch_slot(&mut next_dispatch_at, dispatch_spacing).await;
        spawn_placement_batch(&mut in_flight, Arc::clone(&client), batch, runtime.clone());
    }

    while let Some(join_result) = in_flight.join_next().await {
        let (batch_index, placements, batch_usage) = match join_result {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => return Err(err),
            Err(err) => {
                return Err(AppError::Execution(format!(
                    "placement batch task failed: {err}"
                )));
            }
        };
        batch_results.push((batch_index, placements));
        usage.merge(&batch_usage);
        progress.inc(1);

        if let Some(batch) = pending_batches.next() {
            wait_for_dispatch_slot(&mut next_dispatch_at, dispatch_spacing).await;
            spawn_placement_batch(&mut in_flight, Arc::clone(&client), batch, runtime.clone());
        }
    }

    batch_results.sort_by_key(|(batch_index, _)| *batch_index);
    progress.finish();
    Ok((batch_results, usage))
}

fn spawn_placement_batch(
    join_set: &mut JoinSet<Result<(usize, Vec<PlacementDecision>, LlmUsageSummary)>>,
    client: Arc<dyn LlmClient>,
    batch: PreparedPlacementBatch,
    runtime: PlacementBatchRuntime,
) {
    join_set.spawn(async move {
        let started_at = Instant::now();
        let batch_span = format_paper_batch_span(&batch.papers);
        runtime.options.verbosity.stage_line(
            "placements",
            format!(
                "batch {}/{} {}",
                batch.batch_index, runtime.total_batches, batch_span
            ),
        );
        let (placements, usage) =
            match generate_placement_batch(client.as_ref(), &batch, &runtime).await {
                Ok(placements) => placements,
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
        runtime.options.verbosity.success_line(
            "PLACEMENTS",
            format!(
                "batch {}/{} completed in {} {}",
                batch.batch_index,
                runtime.total_batches,
                format_duration(started_at.elapsed()),
                batch_span
            ),
        );
        Ok((batch.batch_index, placements, usage))
    });
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
