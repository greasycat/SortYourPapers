mod embedding;
mod engine;
mod llm_tiebreak;
mod progress;
mod scoring;

use std::{
    collections::{HashMap, HashSet},
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
    time::Instant,
};

use paper_db::{PaperDb, ReferenceMatchRecord, ReferencePaperInput, ReferenceSetInput};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    error::{AppError, Result},
    llm::LlmUsageSummary,
    llm::{JsonResponseSchema, LlmClient, build_embedding_client, call_json_with_retry},
    papers::KeywordSet,
    papers::{
        PaperText, PreliminaryCategoryPair,
        embedding_support::{
            PaperDbEmbeddingAdapter, build_embedding_config, embedding_model_id, map_paperdb_error,
            paper_input_from, usage_from_db_metrics,
        },
        taxonomy::CategoryTree,
    },
    terminal::{ProgressTracker, format_duration},
    testsets::{CuratedPaper, load_manifest_from_path},
};

use super::{
    MAX_JSON_ATTEMPTS, MAX_SEMANTIC_ATTEMPTS, OutputSnapshot, PaperPlacementEvidence,
    PlacementAssistance, PlacementBatchProgress, PlacementBatchResult, PlacementBatchRuntime,
    PlacementCandidateScore, PlacementDecision, PlacementDecisionSource, PlacementEmbeddingOptions,
    PlacementEvidence, PlacementOptions, PlacementReferenceSupport, PlacementTargetProfile,
    PlacementTargetProfileSource,
    batching::{batch_dispatch_spacing, wait_for_dispatch_slot},
    prompts::{
        build_allowed_targets, build_file_context, build_placement_prompt, format_paper_batch_span,
        format_placement_request_debug_message,
    },
    validation::validate_placements,
};

const DEFAULT_ROOT_TARGET_TEXT: &str = "root";

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

#[derive(Debug, Clone)]
struct PlacementEmbeddingRuntime {
    allowed_targets: Vec<String>,
    target_profiles: Vec<PlacementTargetProfile>,
    target_embeddings: HashMap<String, Vec<f32>>,
    paper_embeddings: HashMap<String, Vec<f32>>,
    candidate_top_k: usize,
    min_similarity: f32,
    min_margin: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct PlacementRunResult {
    pub(crate) placements: Vec<PlacementDecision>,
    pub(crate) usage: LlmUsageSummary,
    pub(crate) evidence: PlacementEvidence,
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
    let result = generate_placements_with_progress(
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
    .await?;
    Ok((result.placements, result.usage))
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
) -> Result<PlacementRunResult>
where
    F: FnMut(&PlacementBatchProgress) -> Result<()>,
{
    if papers.is_empty() {
        return Ok(PlacementRunResult {
            placements: Vec::new(),
            usage: LlmUsageSummary::default(),
            evidence: PlacementEvidence::empty(options.assistance),
        });
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
    let allowed_targets = build_allowed_targets(
        categories,
        snapshot,
        options.placement_mode,
        options.category_depth,
    );
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
    let mut progress_state = progress::validate_saved_placement_progress(
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
        options: options.clone(),
        total_batches,
    };
    let (embedding_runtime, embedding_usage) = match options.assistance {
        PlacementAssistance::LlmOnly => (None, LlmUsageSummary::default()),
        PlacementAssistance::EmbeddingPrimary => {
            let embedding_options = options.embedding.as_ref().ok_or_else(|| {
                AppError::Execution("missing embedding placement configuration".to_string())
            })?;
            let (prepared, usage) = embedding::prepare_embedding_runtime(
                papers,
                &allowed_targets,
                embedding_options,
                options.verbosity,
            )
            .await?;
            (Some(prepared), usage)
        }
    };
    let completed_indexes = progress_state
        .completed_batches
        .iter()
        .map(|batch| batch.batch_index)
        .collect::<HashSet<_>>();
    let mut usage = progress_state.usage.clone();
    usage.merge(&embedding_usage);
    if embedding_usage.has_activity() {
        progress_state.usage = usage.clone();
        on_progress(&progress_state)?;
    }
    let mut next_dispatch_at = None;
    let dispatch_spacing = batch_dispatch_spacing(runtime.options.batch_start_delay_ms);
    let mut progress_tracker = ProgressTracker::new(
        runtime.options.verbosity,
        total_batches,
        "placement batches",
        true,
    );
    progress_tracker.inc(progress_state.completed_batches.len());

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
        let (placements, batch_usage, evidence) =
            match engine::generate_placement_batch(client.as_ref(), batch, &runtime, embedding_runtime.as_ref())
                .await
            {
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
            evidence,
            elapsed_ms: elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
        });
        progress_state
            .completed_batches
            .sort_by_key(|saved_batch| saved_batch.batch_index);
        progress_state.usage = usage.clone();
        on_progress(&progress_state)?;
        progress_tracker.inc(1);
    }
    progress_tracker.finish();

    let mut all_placements = Vec::with_capacity(papers.len());
    let mut all_evidence = Vec::with_capacity(papers.len());
    for batch in &progress_state.completed_batches {
        all_placements.extend(batch.placements.clone());
        all_evidence.extend(batch.evidence.clone());
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

    Ok(PlacementRunResult {
        placements: all_placements,
        usage,
        evidence: PlacementEvidence {
            assistance: options.assistance,
            target_profiles: embedding_runtime
                .as_ref()
                .map(|runtime| runtime.target_profiles.clone())
                .unwrap_or_default(),
            papers: all_evidence,
        },
    })
}

#[cfg(test)]
mod tests;
