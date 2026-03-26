use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use serde::Deserialize;
use tokio::time::timeout;

use crate::{
    error::{AppError, Result},
    llm::LlmUsageSummary,
    llm::{
        JsonResponseSchema, LlmClient, call_json_with_retry, call_text_with_retry, strip_code_fence,
    },
    papers::PreliminaryCategoryPair,
    papers::taxonomy::{CategoryTree, TaxonomyReferenceEvidence},
    terminal::{ProgressTracker, Verbosity, format_duration},
};

use super::{
    GLOBAL_TAXONOMY_LABEL, MAX_JSON_ATTEMPTS, MAX_SEMANTIC_ATTEMPTS, TaxonomyBatchProgress,
    TaxonomyBatchResult,
    batching::{batch_dispatch_spacing, wait_for_dispatch_slot},
    prompts::{
        build_category_prompt, build_merge_category_plain_text_prompt, build_merge_category_prompt,
        format_llm_request_debug_message,
    },
    validation::{
        aggregate_preliminary_categories, validate_category_depth, validate_category_names,
    },
};

const TAXONOMY_MERGE_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Deserialize)]
struct CategoryResponse {
    categories: Vec<Vec<String>>,
}

#[derive(Debug, Clone)]
struct PreparedTaxonomyBatch {
    batch_index: usize,
    aggregated_preliminary_categories: Vec<(String, usize)>,
}

/// Synthesizes a global folder taxonomy from aggregated preliminary category
/// text extracted for each paper.
///
/// # Errors
/// Returns an error when the LLM request fails or the returned taxonomy does
/// not satisfy the configured depth and category-name validation rules.
pub async fn synthesize_categories(
    client: &dyn LlmClient,
    preliminary_pairs: &[PreliminaryCategoryPair],
    category_depth: u8,
    taxonomy_batch_size: usize,
    batch_start_delay_ms: u64,
    subcategories_suggestion_number: usize,
    verbosity: Verbosity,
) -> Result<(Vec<CategoryTree>, LlmUsageSummary)> {
    let batch_progress = synthesize_category_batches_with_progress(
        client,
        preliminary_pairs,
        category_depth,
        taxonomy_batch_size,
        batch_start_delay_ms,
        TaxonomyBatchProgress::default(),
        |_| Ok(()),
        verbosity,
    )
    .await?;
    let partial_categories = batch_progress
        .completed_batches
        .iter()
        .map(|batch| batch.categories.clone())
        .collect::<Vec<_>>();
    let (categories, merge_usage) = merge_category_batches(
        client,
        &partial_categories,
        category_depth,
        subcategories_suggestion_number,
        None,
        None,
        None,
        verbosity,
    )
    .await?;
    let mut usage = batch_progress.usage;
    usage.merge(&merge_usage);
    Ok((categories, usage))
}

#[cfg_attr(not(test), allow(dead_code))]
pub async fn synthesize_categories_with_progress<F>(
    client: &dyn LlmClient,
    preliminary_pairs: &[PreliminaryCategoryPair],
    category_depth: u8,
    taxonomy_batch_size: usize,
    batch_start_delay_ms: u64,
    subcategories_suggestion_number: usize,
    saved_progress: TaxonomyBatchProgress,
    on_progress: F,
    verbosity: Verbosity,
) -> Result<(Vec<CategoryTree>, LlmUsageSummary)>
where
    F: FnMut(&TaxonomyBatchProgress) -> Result<()>,
{
    let batch_progress = synthesize_category_batches_with_progress(
        client,
        preliminary_pairs,
        category_depth,
        taxonomy_batch_size,
        batch_start_delay_ms,
        saved_progress,
        on_progress,
        verbosity,
    )
    .await?;
    let partial_categories = batch_progress
        .completed_batches
        .iter()
        .map(|batch| batch.categories.clone())
        .collect::<Vec<_>>();
    let (categories, merge_usage) = merge_category_batches(
        client,
        &partial_categories,
        category_depth,
        subcategories_suggestion_number,
        None,
        None,
        None,
        verbosity,
    )
    .await?;
    let mut usage = batch_progress.usage;
    usage.merge(&merge_usage);
    Ok((categories, usage))
}

pub async fn synthesize_category_batches_with_progress<F>(
    client: &dyn LlmClient,
    preliminary_pairs: &[PreliminaryCategoryPair],
    category_depth: u8,
    taxonomy_batch_size: usize,
    batch_start_delay_ms: u64,
    saved_progress: TaxonomyBatchProgress,
    mut on_progress: F,
    verbosity: Verbosity,
) -> Result<TaxonomyBatchProgress>
where
    F: FnMut(&TaxonomyBatchProgress) -> Result<()>,
{
    if preliminary_pairs.is_empty() {
        return Ok(TaxonomyBatchProgress::default());
    }
    if taxonomy_batch_size == 0 {
        return Err(AppError::Validation(
            "taxonomy_batch_size must be greater than 0".to_string(),
        ));
    }

    let aggregated_preliminary_categories = aggregate_preliminary_categories(preliminary_pairs);
    let prepared_batches = aggregated_preliminary_categories
        .chunks(taxonomy_batch_size)
        .enumerate()
        .map(|(index, batch)| PreparedTaxonomyBatch {
            batch_index: index + 1,
            aggregated_preliminary_categories: batch.to_vec(),
        })
        .collect::<Vec<_>>();
    let mut progress_state = validate_saved_taxonomy_progress(&prepared_batches, saved_progress)?;
    let resumed_batches = progress_state.completed_batches.len();
    if resumed_batches > 0 {
        verbosity.stage_line(
            "taxonomy",
            format!("resuming {} saved taxonomy batch(es)", resumed_batches),
        );
    }

    verbosity.stage_line(
        "taxonomy",
        format!(
            "synthesizing categories from {} aggregated preliminary categor(ies) across {} batch(es) with max depth {}",
            aggregated_preliminary_categories.len(),
            prepared_batches.len(),
            category_depth
        ),
    );

    let mut progress = ProgressTracker::new(verbosity, prepared_batches.len(), "taxonomy", true);
    progress.inc(progress_state.completed_batches.len());
    let completed_indexes = progress_state
        .completed_batches
        .iter()
        .map(|batch| batch.batch_index)
        .collect::<HashSet<_>>();
    let mut usage = progress_state.usage.clone();
    let mut next_dispatch_at = None;
    let dispatch_spacing = batch_dispatch_spacing(batch_start_delay_ms);

    for batch in prepared_batches
        .iter()
        .filter(|batch| !completed_indexes.contains(&batch.batch_index))
    {
        wait_for_dispatch_slot(&mut next_dispatch_at, dispatch_spacing).await;
        let started_at = Instant::now();
        let label = format!("taxonomy/batch {}", batch.batch_index);
        let batch_user =
            build_category_prompt(&batch.aggregated_preliminary_categories, category_depth)?;
        let (categories, batch_usage) = request_validated_categories(
            client,
            "You design hierarchical folder taxonomies for academic PDFs. Return strict JSON only.",
            &batch_user,
            category_depth,
            verbosity,
            &label,
        )
        .await?;
        let elapsed = started_at.elapsed();
        let elapsed_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
        verbosity.stage_line(
            "taxonomy",
            format!(
                "batch {}/{} completed in {} from {} aggregated categor(ies)",
                batch.batch_index,
                prepared_batches.len(),
                format_duration(elapsed),
                batch.aggregated_preliminary_categories.len()
            ),
        );
        usage.merge(&batch_usage);
        progress_state.completed_batches.push(TaxonomyBatchResult {
            batch_index: batch.batch_index,
            input_count: batch.aggregated_preliminary_categories.len(),
            input_fingerprint: Some(taxonomy_batch_fingerprint(
                &batch.aggregated_preliminary_categories,
            )?),
            categories,
            elapsed_ms,
        });
        progress_state
            .completed_batches
            .sort_by_key(|batch| batch.batch_index);
        progress_state.usage = usage.clone();
        on_progress(&progress_state)?;
        progress.inc(1);
    }

    report_slowest_batch(
        &progress_state.completed_batches,
        prepared_batches.len(),
        verbosity,
    );

    progress.finish();
    Ok(progress_state)
}

pub async fn merge_category_batches(
    client: &dyn LlmClient,
    partial_categories: &[Vec<CategoryTree>],
    category_depth: u8,
    subcategories_suggestion_number: usize,
    user_suggestion: Option<&str>,
    existing_output_folders: Option<&[String]>,
    reference_evidence: Option<&TaxonomyReferenceEvidence>,
    verbosity: Verbosity,
) -> Result<(Vec<CategoryTree>, LlmUsageSummary)> {
    if partial_categories.is_empty() {
        return Ok((Vec::new(), LlmUsageSummary::default()));
    }

    if partial_categories.len() == 1 && user_suggestion.is_none() {
        return Ok((
            partial_categories.first().cloned().unwrap_or_default(),
            LlmUsageSummary::default(),
        ));
    }

    if client.prefers_plain_text_taxonomy_merge() {
        return request_plain_text_merged_categories(
            client,
            partial_categories,
            category_depth,
            subcategories_suggestion_number,
            user_suggestion,
            existing_output_folders,
            reference_evidence,
            verbosity,
            GLOBAL_TAXONOMY_LABEL,
        )
        .await;
    }

    merge_category_batches_with_timeout(
        client,
        partial_categories,
        category_depth,
        subcategories_suggestion_number,
        user_suggestion,
        existing_output_folders,
        reference_evidence,
        verbosity,
        Duration::from_secs(TAXONOMY_MERGE_TIMEOUT_SECS),
    )
    .await
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) async fn merge_category_batches_with_timeout(
    client: &dyn LlmClient,
    partial_categories: &[Vec<CategoryTree>],
    category_depth: u8,
    subcategories_suggestion_number: usize,
    user_suggestion: Option<&str>,
    existing_output_folders: Option<&[String]>,
    reference_evidence: Option<&TaxonomyReferenceEvidence>,
    verbosity: Verbosity,
    merge_timeout: Duration,
) -> Result<(Vec<CategoryTree>, LlmUsageSummary)> {
    let merge_user = build_merge_category_prompt(
        partial_categories,
        category_depth,
        subcategories_suggestion_number,
        user_suggestion,
        existing_output_folders,
        reference_evidence,
    )?;
    match timeout(
        merge_timeout,
        request_validated_categories_json(
            client,
            "You merge partial folder taxonomies for academic PDFs into one final taxonomy. Return strict JSON only.",
            &merge_user,
            category_depth,
            verbosity,
            GLOBAL_TAXONOMY_LABEL,
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            verbosity.warn_line(
                "TAXONOMY",
                format!(
                    "{} structured merge exceeded {}; retrying with plain-text paths",
                    GLOBAL_TAXONOMY_LABEL,
                    format_duration(merge_timeout)
                ),
            );
            request_plain_text_merged_categories(
                client,
                partial_categories,
                category_depth,
                subcategories_suggestion_number,
                user_suggestion,
                existing_output_folders,
                reference_evidence,
                verbosity,
                GLOBAL_TAXONOMY_LABEL,
            )
            .await
        }
    }
}

async fn request_plain_text_merged_categories(
    client: &dyn LlmClient,
    partial_categories: &[Vec<CategoryTree>],
    category_depth: u8,
    subcategories_suggestion_number: usize,
    user_suggestion: Option<&str>,
    existing_output_folders: Option<&[String]>,
    reference_evidence: Option<&TaxonomyReferenceEvidence>,
    verbosity: Verbosity,
    label: &str,
) -> Result<(Vec<CategoryTree>, LlmUsageSummary)> {
    let fallback_user = build_merge_category_plain_text_prompt(
        partial_categories,
        category_depth,
        subcategories_suggestion_number,
        user_suggestion,
        existing_output_folders,
        reference_evidence,
    )?;
    request_validated_categories_plain_text(
        client,
        "You merge partial folder taxonomies for academic PDFs into one final taxonomy. Return plain text only.",
        &fallback_user,
        category_depth,
        verbosity,
        label,
    )
    .await
}

async fn request_validated_categories(
    client: &dyn LlmClient,
    system: &str,
    base_user: &str,
    category_depth: u8,
    verbosity: Verbosity,
    label: &str,
) -> Result<(Vec<CategoryTree>, LlmUsageSummary)> {
    request_validated_categories_json(client, system, base_user, category_depth, verbosity, label)
        .await
}

async fn request_validated_categories_json(
    client: &dyn LlmClient,
    system: &str,
    base_user: &str,
    category_depth: u8,
    verbosity: Verbosity,
    label: &str,
) -> Result<(Vec<CategoryTree>, LlmUsageSummary)> {
    let mut user = base_user.to_string();
    let mut usage = LlmUsageSummary::default();
    let mut last_issue = String::new();

    for attempt in 1..=MAX_SEMANTIC_ATTEMPTS {
        if verbosity.debug_enabled() {
            verbosity.debug_line(
                "LLM",
                format_llm_request_debug_message(label, attempt, system, &user),
            );
        }

        let schema = category_response_schema();
        let mut response: crate::llm::ParsedLlmResponse<CategoryResponse> =
            call_json_with_retry(client, system, &user, &schema, MAX_JSON_ATTEMPTS).await?;

        let categories = match validate_requested_categories(
            &response.value.categories,
            category_depth,
        ) {
            Ok(categories) => categories,
            Err(err) => {
                last_issue = err.to_string();
                if attempt < MAX_SEMANTIC_ATTEMPTS {
                    response.metrics.semantic_retry_count += 1;
                }
                usage.record_call(&response.metrics);
                if attempt < MAX_SEMANTIC_ATTEMPTS {
                    verbosity.warn_line(
                        "RETRY",
                        format!(
                            "{} retry {}/{}: {}",
                            label,
                            attempt + 1,
                            MAX_SEMANTIC_ATTEMPTS,
                            last_issue
                        ),
                    );
                    user = format!(
                        "{base_user}\n\nYour previous response failed validation with this error: {last_issue}.\nReturn corrected JSON that satisfies all rules."
                    );
                }
                continue;
            }
        };

        usage.record_call(&response.metrics);
        return Ok((categories, usage));
    }

    Err(AppError::Validation(format!(
        "failed category synthesis validation: {last_issue}"
    )))
}

async fn request_validated_categories_plain_text(
    client: &dyn LlmClient,
    system: &str,
    base_user: &str,
    category_depth: u8,
    verbosity: Verbosity,
    label: &str,
) -> Result<(Vec<CategoryTree>, LlmUsageSummary)> {
    let mut user = base_user.to_string();
    let mut usage = LlmUsageSummary::default();
    let mut last_issue = String::new();

    for attempt in 1..=MAX_SEMANTIC_ATTEMPTS {
        if verbosity.debug_enabled() {
            verbosity.debug_line(
                "LLM",
                format_llm_request_debug_message(label, attempt, system, &user),
            );
        }

        let mut response = call_text_with_retry(client, system, &user).await?;
        let category_paths = match parse_plain_text_category_paths(&response.content) {
            Ok(paths) => paths,
            Err(err) => {
                last_issue = err.to_string();
                if attempt < MAX_SEMANTIC_ATTEMPTS {
                    response.metrics.semantic_retry_count += 1;
                }
                usage.record_call(&response.metrics);
                if attempt < MAX_SEMANTIC_ATTEMPTS {
                    verbosity.warn_line(
                        "RETRY",
                        format!(
                            "{} retry {}/{}: {}",
                            label,
                            attempt + 1,
                            MAX_SEMANTIC_ATTEMPTS,
                            last_issue
                        ),
                    );
                    user = format!(
                        "{base_user}\n\nYour previous response failed validation with this error: {last_issue}.\nReturn corrected plain text that satisfies all rules.\nImportant: return one full category path per line, use ` > ` between segments, and do not return JSON or markdown."
                    );
                }
                continue;
            }
        };

        let categories = match validate_requested_categories(&category_paths, category_depth) {
            Ok(categories) => categories,
            Err(err) => {
                last_issue = err.to_string();
                if attempt < MAX_SEMANTIC_ATTEMPTS {
                    response.metrics.semantic_retry_count += 1;
                }
                usage.record_call(&response.metrics);
                if attempt < MAX_SEMANTIC_ATTEMPTS {
                    verbosity.warn_line(
                        "RETRY",
                        format!(
                            "{} retry {}/{}: {}",
                            label,
                            attempt + 1,
                            MAX_SEMANTIC_ATTEMPTS,
                            last_issue
                        ),
                    );
                    user = format!(
                        "{base_user}\n\nYour previous response failed validation with this error: {last_issue}.\nReturn corrected plain text that satisfies all rules.\nImportant: return one full category path per line, use ` > ` between segments, and do not return JSON or markdown."
                    );
                }
                continue;
            }
        };

        usage.record_call(&response.metrics);
        return Ok((categories, usage));
    }

    Err(AppError::Validation(format!(
        "failed category synthesis validation: {last_issue}"
    )))
}

fn category_response_schema() -> JsonResponseSchema {
    JsonResponseSchema::new(
        "category_response",
        serde_json::json!({
            "type": "object",
            "properties": {
                "categories": {
                    "type": "array",
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        }
                    }
                }
            },
            "required": ["categories"],
            "additionalProperties": false
        }),
    )
}

fn validate_requested_categories(
    paths: &[Vec<String>],
    category_depth: u8,
) -> Result<Vec<CategoryTree>> {
    let categories = rebuild_category_tree(paths)?;

    if categories.is_empty() {
        return Err(AppError::Validation(
            "category synthesis returned no categories".to_string(),
        ));
    }
    validate_category_depth(&categories, category_depth)?;
    validate_category_names(&categories)?;
    Ok(categories)
}

pub(super) fn parse_plain_text_category_paths(raw: &str) -> Result<Vec<Vec<String>>> {
    let paths = strip_code_fence(raw)
        .lines()
        .map(str::trim)
        .filter(|line: &&str| !line.is_empty())
        .map(|line: &str| {
            line.split('>')
                .map(|segment: &str| segment.trim().to_string())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    validate_category_paths(&paths)?;
    Ok(paths)
}

fn rebuild_category_tree(paths: &[Vec<String>]) -> Result<Vec<CategoryTree>> {
    validate_category_paths(paths)?;

    let mut categories = Vec::new();
    for path in paths {
        insert_category_path(&mut categories, path);
    }

    Ok(categories)
}

fn validate_category_paths(paths: &[Vec<String>]) -> Result<()> {
    let mut seen_paths = HashSet::new();

    for (index, path) in paths.iter().enumerate() {
        if path.is_empty() {
            return Err(AppError::Validation(format!(
                "category path at index {} is empty",
                index
            )));
        }

        for segment in path {
            if segment.trim().is_empty() {
                return Err(AppError::Validation(format!(
                    "category path at index {} contains an empty segment",
                    index
                )));
            }
        }

        if !seen_paths.insert(path.join("\x1f")) {
            return Err(AppError::Validation(format!(
                "duplicate category path at index {}",
                index
            )));
        }
    }

    Ok(())
}

fn insert_category_path(categories: &mut Vec<CategoryTree>, path: &[String]) {
    let Some((head, tail)) = path.split_first() else {
        return;
    };

    if let Some(existing) = categories
        .iter_mut()
        .find(|category| category.name == *head)
    {
        insert_category_path(&mut existing.children, tail);
        return;
    }

    let mut category = CategoryTree {
        name: head.clone(),
        children: Vec::new(),
    };
    insert_category_path(&mut category.children, tail);
    categories.push(category);
}

fn validate_saved_taxonomy_progress(
    prepared_batches: &[PreparedTaxonomyBatch],
    mut progress: TaxonomyBatchProgress,
) -> Result<TaxonomyBatchProgress> {
    if progress.completed_batches.is_empty() {
        return Ok(progress);
    }

    let expected_batches = prepared_batches
        .iter()
        .map(|batch| {
            (
                batch.batch_index,
                (
                    batch.aggregated_preliminary_categories.len(),
                    taxonomy_batch_fingerprint(&batch.aggregated_preliminary_categories),
                ),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();

    for batch in &progress.completed_batches {
        let Some((expected_len, expected_fingerprint)) = expected_batches.get(&batch.batch_index)
        else {
            return Err(AppError::Validation(format!(
                "saved taxonomy batch {} no longer matches the current input",
                batch.batch_index
            )));
        };
        if batch.input_count != *expected_len {
            return Err(AppError::Validation(format!(
                "saved taxonomy batch {} has inconsistent input counts",
                batch.batch_index
            )));
        }
        if let Some(saved_fingerprint) = batch.input_fingerprint.as_ref() {
            let expected_fingerprint = expected_fingerprint.as_ref().map_err(|err| {
                AppError::Execution(format!(
                    "failed to fingerprint taxonomy batch {}: {err}",
                    batch.batch_index
                ))
            })?;
            if saved_fingerprint != expected_fingerprint {
                return Err(AppError::Validation(format!(
                    "saved taxonomy batch {} no longer matches the current input",
                    batch.batch_index
                )));
            }
        }
    }

    progress
        .completed_batches
        .sort_by_key(|batch| batch.batch_index);
    Ok(progress)
}

fn taxonomy_batch_fingerprint(batch: &[(String, usize)]) -> Result<String> {
    serde_json::to_string(batch).map_err(AppError::from)
}

fn report_slowest_batch(
    completed_batches: &[TaxonomyBatchResult],
    total_batches: usize,
    verbosity: Verbosity,
) {
    if completed_batches.len() < 2 {
        return;
    }

    let mut durations = completed_batches
        .iter()
        .map(|batch| batch.elapsed_ms)
        .collect::<Vec<_>>();
    durations.sort_unstable();
    let median_ms = durations[durations.len() / 2];
    let Some(slowest) = completed_batches
        .iter()
        .max_by_key(|batch| batch.elapsed_ms)
    else {
        return;
    };
    if slowest.elapsed_ms < median_ms.saturating_mul(2) || slowest.elapsed_ms < median_ms + 200 {
        return;
    }

    let slowest_duration = std::time::Duration::from_millis(slowest.elapsed_ms);
    let median_duration = std::time::Duration::from_millis(median_ms);
    verbosity.warn_line(
        "TAXONOMY",
        format!(
            "slow batch {}/{} took {} (median {})",
            slowest.batch_index,
            total_batches,
            format_duration(slowest_duration),
            format_duration(median_duration)
        ),
    );
}
