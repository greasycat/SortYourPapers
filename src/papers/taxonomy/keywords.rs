use std::{sync::Arc, time::Instant};

use serde::Deserialize;
use tokio::task::JoinSet;

use crate::{
    error::{AppError, Result},
    llm::LlmUsageSummary,
    llm::{JsonResponseSchema, LlmClient, call_json_with_retry},
    papers::{KeywordSet, KeywordStageState, PaperText, PreliminaryCategoryPair},
    terminal::{ProgressTracker, Verbosity, format_duration},
};

use super::{
    KeywordBatchProgress, KeywordBatchResult, KeywordPair, MAX_CONCURRENT_KEYWORD_BATCH_REQUESTS,
    MAX_JSON_ATTEMPTS, MAX_SEMANTIC_ATTEMPTS,
    batching::{batch_dispatch_spacing, wait_for_dispatch_slot},
    prompts::{build_batch_keyword_prompt, format_batch_span, format_llm_request_debug_message},
    validation::validate_keyword_batch_response,
};

#[derive(Debug, Deserialize)]
struct KeywordBatchResponse {
    pairs: Vec<KeywordPair>,
}

#[derive(Debug, Clone)]
struct PreparedKeywordBatch {
    batch_index: usize,
    papers: Vec<PaperText>,
}

/// Extracts per-paper keywords from LLM-ready paper text batches.
///
/// # Errors
/// Returns an error when batching is misconfigured, an LLM request fails,
/// or the LLM response does not validate against the expected batch shape.
pub async fn extract_keywords(
    client: Arc<dyn LlmClient>,
    papers: &[PaperText],
    keyword_batch_size: usize,
    batch_start_delay_ms: u64,
    verbosity: Verbosity,
) -> Result<(KeywordStageState, LlmUsageSummary)> {
    extract_keywords_with_progress(
        client,
        papers,
        keyword_batch_size,
        batch_start_delay_ms,
        KeywordBatchProgress::default(),
        |_| Ok(()),
        verbosity,
    )
    .await
}

/// Extracts per-paper keywords plus preliminary category text from LLM-ready
/// paper text batches while persisting resumable batch progress.
///
/// # Errors
/// Returns an error when batching is misconfigured, an LLM request fails,
/// or the LLM response does not validate against the expected batch shape.
pub(crate) async fn extract_keywords_with_progress<F>(
    client: Arc<dyn LlmClient>,
    papers: &[PaperText],
    keyword_batch_size: usize,
    batch_start_delay_ms: u64,
    saved_progress: KeywordBatchProgress,
    mut on_progress: F,
    verbosity: Verbosity,
) -> Result<(KeywordStageState, LlmUsageSummary)>
where
    F: FnMut(&KeywordBatchProgress) -> Result<()>,
{
    if papers.is_empty() {
        return Ok((
            KeywordStageState {
                keyword_sets: Vec::new(),
                preliminary_pairs: Vec::new(),
            },
            LlmUsageSummary::default(),
        ));
    }
    if keyword_batch_size == 0 {
        return Err(AppError::Validation(
            "keyword_batch_size must be greater than 0".to_string(),
        ));
    }

    let total_batches = papers.len().div_ceil(keyword_batch_size);
    verbosity.stage_line(
        "keywords",
        format!(
            "{} papers ready; batching into {} request(s) of up to {} file(s)",
            papers.len(),
            total_batches,
            keyword_batch_size
        ),
    );
    let prepared_batches = papers
        .chunks(keyword_batch_size)
        .enumerate()
        .map(|(batch_index, batch)| PreparedKeywordBatch {
            batch_index: batch_index + 1,
            papers: batch.to_vec(),
        })
        .collect::<Vec<_>>();
    let saved_progress = validate_saved_keyword_progress(&prepared_batches, saved_progress)?;
    let resumed_batches = saved_progress.completed_batches.len();
    if resumed_batches > 0 {
        verbosity.stage_line(
            "keywords",
            format!("resuming {} saved keyword batch(es)", resumed_batches),
        );
    }
    let (batch_results, usage) = run_keyword_batches_concurrently(
        client,
        prepared_batches,
        batch_start_delay_ms,
        saved_progress,
        &mut on_progress,
        verbosity,
    )
    .await?;

    let mut keyword_sets = Vec::with_capacity(papers.len());
    let mut preliminary_pairs = Vec::with_capacity(papers.len());
    for batch_result in batch_results {
        keyword_sets.extend(batch_result.keyword_sets);
        preliminary_pairs.extend(batch_result.preliminary_pairs);
    }

    verbosity.success_line(
        "KEYWORDS",
        format!(
            "completed {} batch(es) and collected {} keyword set(s)",
            total_batches,
            keyword_sets.len()
        ),
    );

    Ok((
        KeywordStageState {
            keyword_sets,
            preliminary_pairs,
        },
        usage,
    ))
}

async fn extract_keyword_batch(
    client: &dyn LlmClient,
    batch: &[PaperText],
    verbosity: Verbosity,
    current_batch: usize,
    total_batches: usize,
) -> Result<(
    Vec<KeywordSet>,
    Vec<PreliminaryCategoryPair>,
    LlmUsageSummary,
)> {
    let system = "You extract concise research keywords from academic paper excerpts. Return strict JSON only.";
    let base_user = build_batch_keyword_prompt(batch)?;
    let mut user = base_user.clone();
    let mut accepted: Option<(Vec<KeywordSet>, Vec<PreliminaryCategoryPair>)> = None;
    let mut usage = LlmUsageSummary::default();
    let mut last_issue = String::new();

    for attempt in 1..=MAX_SEMANTIC_ATTEMPTS {
        if verbosity.debug_enabled() {
            verbosity.debug_line(
                "LLM",
                format_llm_request_debug_message(
                    &format!("keywords batch {current_batch}/{total_batches}"),
                    attempt,
                    system,
                    &user,
                ),
            );
        }

        let schema = keyword_batch_response_schema();
        let mut response: crate::llm::ParsedLlmResponse<KeywordBatchResponse> =
            call_json_with_retry(client, system, &user, &schema, MAX_JSON_ATTEMPTS).await?;

        match validate_keyword_batch_response(&response.value.pairs, batch) {
            Ok(batch_output) => {
                usage.record_call(&response.metrics);
                accepted = Some(batch_output);
                break;
            }
            Err(err) => {
                if attempt < MAX_SEMANTIC_ATTEMPTS {
                    response.metrics.semantic_retry_count += 1;
                }
                usage.record_call(&response.metrics);
                last_issue = err.to_string();
            }
        }

        if attempt < MAX_SEMANTIC_ATTEMPTS {
            verbosity.warn_line(
                "KEYWORDS",
                format!(
                    "batch {current_batch}/{total_batches} retry {}/{}: {}",
                    attempt + 1,
                    MAX_SEMANTIC_ATTEMPTS,
                    last_issue
                ),
            );
            user = format!(
                "{base_user}\n\nYour previous response had this issue: {last_issue}.\nReturn JSON again.\nImportant: return exactly one pair for every file_id."
            );
        }
    }

    let batch_span = format_batch_span(batch);
    accepted
        .map(|(keyword_sets, preliminary_pairs)| (keyword_sets, preliminary_pairs, usage))
        .ok_or_else(|| {
            AppError::Validation(format!(
                "failed keyword extraction validation for batch {batch_span}: {last_issue}"
            ))
        })
}

async fn run_keyword_batches_concurrently(
    client: Arc<dyn LlmClient>,
    prepared_batches: Vec<PreparedKeywordBatch>,
    batch_start_delay_ms: u64,
    mut progress_state: KeywordBatchProgress,
    mut on_progress: impl FnMut(&KeywordBatchProgress) -> Result<()>,
    verbosity: Verbosity,
) -> Result<(Vec<KeywordBatchResult>, LlmUsageSummary)> {
    let total_batches = prepared_batches.len() + progress_state.completed_batches.len();
    let batch_verbosity = if verbosity.show_progress(total_batches, true) {
        verbosity.stage_silenced()
    } else {
        verbosity
    };
    let max_in_flight = MAX_CONCURRENT_KEYWORD_BATCH_REQUESTS.max(1);
    let dispatch_spacing = batch_dispatch_spacing(batch_start_delay_ms);
    let completed_indexes = progress_state
        .completed_batches
        .iter()
        .map(|batch| batch.batch_index)
        .collect::<std::collections::HashSet<_>>();
    let mut pending_batches = prepared_batches.into_iter();
    let mut in_flight = JoinSet::new();
    let mut next_dispatch_at = None;
    let mut progress = ProgressTracker::new(verbosity, total_batches, "keyword batches", true);
    progress.inc(progress_state.completed_batches.len());

    for _ in 0..max_in_flight {
        let Some(batch) =
            pending_batches.find(|batch| !completed_indexes.contains(&batch.batch_index))
        else {
            break;
        };
        wait_for_dispatch_slot(&mut next_dispatch_at, dispatch_spacing).await;
        spawn_keyword_batch(
            &mut in_flight,
            Arc::clone(&client),
            batch,
            batch_verbosity,
            total_batches,
        );
    }

    while let Some(join_result) = in_flight.join_next().await {
        let (batch_result, batch_usage) = match join_result {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => return Err(err),
            Err(err) => {
                return Err(AppError::Execution(format!(
                    "keyword batch task failed: {err}"
                )));
            }
        };
        progress_state.completed_batches.push(batch_result);
        progress_state
            .completed_batches
            .sort_by_key(|batch| batch.batch_index);
        progress_state.usage.merge(&batch_usage);
        on_progress(&progress_state)?;
        progress.inc(1);

        if let Some(batch) =
            pending_batches.find(|batch| !completed_indexes.contains(&batch.batch_index))
        {
            wait_for_dispatch_slot(&mut next_dispatch_at, dispatch_spacing).await;
            spawn_keyword_batch(
                &mut in_flight,
                Arc::clone(&client),
                batch,
                batch_verbosity,
                total_batches,
            );
        }
    }

    progress.finish();
    Ok((progress_state.completed_batches, progress_state.usage))
}

fn spawn_keyword_batch(
    join_set: &mut JoinSet<Result<(KeywordBatchResult, LlmUsageSummary)>>,
    client: Arc<dyn LlmClient>,
    batch: PreparedKeywordBatch,
    verbosity: Verbosity,
    total_batches: usize,
) {
    join_set.spawn(async move {
        let started_at = Instant::now();
        let batch_span = format_batch_span(&batch.papers);
        verbosity.stage_line(
            "keywords",
            format!(
                "batch {}/{} {}",
                batch.batch_index, total_batches, batch_span
            ),
        );
        let (keyword_sets, preliminary_pairs, usage) = match extract_keyword_batch(
            client.as_ref(),
            &batch.papers,
            verbosity,
            batch.batch_index,
            total_batches,
        )
        .await
        {
            Ok(keyword_sets) => keyword_sets,
            Err(err) => {
                verbosity.error_line(
                    "KEYWORDS",
                    format!(
                        "batch {}/{} failed after {} {}: {}",
                        batch.batch_index,
                        total_batches,
                        format_duration(started_at.elapsed()),
                        batch_span,
                        err
                    ),
                );
                return Err(err);
            }
        };
        verbosity.success_line(
            "KEYWORDS",
            format!(
                "batch {}/{} completed in {} {}",
                batch.batch_index,
                total_batches,
                format_duration(started_at.elapsed()),
                batch_span
            ),
        );
        Ok((
            KeywordBatchResult {
                batch_index: batch.batch_index,
                keyword_sets,
                preliminary_pairs,
            },
            usage,
        ))
    });
}

fn keyword_batch_response_schema() -> JsonResponseSchema {
    JsonResponseSchema::new(
        "keyword_batch_response",
        serde_json::json!({
            "type": "object",
            "properties": {
                "pairs": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "file_id": {
                                "type": "string"
                            },
                            "keywords": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                }
                            },
                            "preliminary_categories_k_depth": {
                                "type": "string"
                            }
                        },
                        "required": ["file_id", "keywords", "preliminary_categories_k_depth"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["pairs"],
            "additionalProperties": false
        }),
    )
}

fn validate_saved_keyword_progress(
    prepared_batches: &[PreparedKeywordBatch],
    mut progress: KeywordBatchProgress,
) -> Result<KeywordBatchProgress> {
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
        .collect::<std::collections::HashMap<_, _>>();

    for batch in &progress.completed_batches {
        let Some(expected_file_ids) = expected_batches.get(&batch.batch_index) else {
            return Err(AppError::Validation(format!(
                "saved keyword batch {} no longer matches the current input",
                batch.batch_index
            )));
        };
        let keyword_file_ids = batch
            .keyword_sets
            .iter()
            .map(|set| set.file_id.clone())
            .collect::<Vec<_>>();
        let preliminary_file_ids = batch
            .preliminary_pairs
            .iter()
            .map(|pair| pair.file_id.clone())
            .collect::<Vec<_>>();
        if keyword_file_ids != *expected_file_ids || preliminary_file_ids != *expected_file_ids {
            return Err(AppError::Validation(format!(
                "saved keyword batch {} has inconsistent file ids",
                batch.batch_index
            )));
        }
    }

    progress
        .completed_batches
        .sort_by_key(|batch| batch.batch_index);
    Ok(progress)
}
