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
        options: options.clone(),
        total_batches,
    };
    let (embedding_runtime, embedding_usage) = match options.assistance {
        PlacementAssistance::LlmOnly => (None, LlmUsageSummary::default()),
        PlacementAssistance::EmbeddingPrimary => {
            let embedding_options = options.embedding.as_ref().ok_or_else(|| {
                AppError::Execution("missing embedding placement configuration".to_string())
            })?;
            let (prepared, usage) = prepare_embedding_runtime(
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
        let (placements, batch_usage, evidence) = match generate_placement_batch(
            client.as_ref(),
            batch,
            &runtime,
            embedding_runtime.as_ref(),
        )
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
        progress.inc(1);
    }
    progress.finish();

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

async fn generate_placement_batch(
    client: &dyn LlmClient,
    batch: &PreparedPlacementBatch,
    runtime: &PlacementBatchRuntime,
    embedding_runtime: Option<&PlacementEmbeddingRuntime>,
) -> Result<(
    Vec<PlacementDecision>,
    LlmUsageSummary,
    Vec<PaperPlacementEvidence>,
)> {
    if let Some(embedding_runtime) = embedding_runtime {
        return generate_embedding_primary_batch(client, batch, runtime, embedding_runtime).await;
    }

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
                return Ok((
                    response.value.placements.clone(),
                    usage,
                    response
                        .value
                        .placements
                        .into_iter()
                        .map(|placement| PaperPlacementEvidence {
                            file_id: placement.file_id,
                            chosen_target_rel_path: placement.target_rel_path,
                            decision_source: PlacementDecisionSource::LlmOnly,
                            top_candidates: Vec::new(),
                            top_score: None,
                            margin_over_runner_up: None,
                        })
                        .collect(),
                ));
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

async fn generate_embedding_primary_batch(
    client: &dyn LlmClient,
    batch: &PreparedPlacementBatch,
    runtime: &PlacementBatchRuntime,
    embedding_runtime: &PlacementEmbeddingRuntime,
) -> Result<(
    Vec<PlacementDecision>,
    LlmUsageSummary,
    Vec<PaperPlacementEvidence>,
)> {
    let mut placements = Vec::with_capacity(batch.papers.len());
    let mut evidence = Vec::with_capacity(batch.papers.len());
    let mut usage = LlmUsageSummary::default();

    for (paper, base_context) in batch.papers.iter().zip(batch.file_context.iter()) {
        let ranking = rank_targets_for_paper(paper, embedding_runtime)?;
        let (top_score, margin_over_runner_up) = top_score_and_margin(&ranking);
        let top_candidates = ranking
            .iter()
            .take(embedding_runtime.candidate_top_k)
            .cloned()
            .collect::<Vec<_>>();

        if should_use_embedding_decision(&ranking, embedding_runtime) {
            let chosen_target_rel_path = ranking
                .first()
                .map(|candidate| candidate.target_rel_path.clone())
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "no placement targets available for {}",
                        paper.file_id
                    ))
                })?;
            placements.push(PlacementDecision {
                file_id: paper.file_id.clone(),
                target_rel_path: chosen_target_rel_path.clone(),
            });
            evidence.push(PaperPlacementEvidence {
                file_id: paper.file_id.clone(),
                chosen_target_rel_path,
                decision_source: PlacementDecisionSource::Embedding,
                top_candidates,
                top_score,
                margin_over_runner_up,
            });
            continue;
        }

        let (placement, tie_break_usage) =
            generate_tiebreak_placement(client, paper, base_context, &top_candidates, runtime)
                .await?;
        usage.merge(&tie_break_usage);
        placements.push(placement.clone());
        evidence.push(PaperPlacementEvidence {
            file_id: paper.file_id.clone(),
            chosen_target_rel_path: placement.target_rel_path,
            decision_source: PlacementDecisionSource::LlmTiebreak,
            top_candidates,
            top_score,
            margin_over_runner_up,
        });
    }

    validate_placements(
        &placements,
        &batch.papers,
        runtime.snapshot.as_ref(),
        runtime.options.placement_mode,
        runtime.options.category_depth,
    )?;
    Ok((placements, usage, evidence))
}

async fn generate_tiebreak_placement(
    client: &dyn LlmClient,
    paper: &PaperText,
    base_context: &Value,
    top_candidates: &[PlacementCandidateScore],
    runtime: &PlacementBatchRuntime,
) -> Result<(PlacementDecision, LlmUsageSummary)> {
    let system = "You assign PDFs to category folders. Return strict JSON only.";
    let allowed_targets = top_candidates
        .iter()
        .map(|candidate| candidate.target_rel_path.clone())
        .collect::<Vec<_>>();
    if allowed_targets.is_empty() {
        return Err(AppError::Validation(format!(
            "no embedding-ranked placement candidates available for {}",
            paper.file_id
        )));
    }

    let file_context = vec![add_embedding_candidates_to_context(
        base_context,
        top_candidates,
    )];
    let base_user = build_placement_prompt(&file_context, &allowed_targets)?;
    let mut user = base_user.clone();
    let mut last_issue = String::new();
    let mut usage = LlmUsageSummary::default();

    for attempt in 1..=MAX_SEMANTIC_ATTEMPTS {
        if runtime.options.verbosity.debug_enabled() {
            runtime.options.verbosity.debug_line(
                "LLM",
                format!(
                    "placement tie-break for {}\n{}",
                    paper.file_id,
                    format_placement_request_debug_message(system, &user)
                ),
            );
        }

        let schema = placement_response_schema();
        let mut response: crate::llm::ParsedLlmResponse<PlacementResponse> =
            call_json_with_retry(client, system, &user, &schema, MAX_JSON_ATTEMPTS).await?;
        if let Some(issue) =
            validate_tiebreak_response(&response.value.placements, paper, &allowed_targets)
        {
            last_issue = issue;
        } else {
            usage.record_call(&response.metrics);
            let placement = response
                .value
                .placements
                .into_iter()
                .next()
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "missing placement decision for {}",
                        paper.file_id
                    ))
                })?;
            return Ok((placement, usage));
        }

        if attempt < MAX_SEMANTIC_ATTEMPTS {
            response.metrics.semantic_retry_count += 1;
        }
        usage.record_call(&response.metrics);

        if attempt < MAX_SEMANTIC_ATTEMPTS {
            user = format!(
                "{base_user}\n\nYour previous response had this issue: {last_issue}.\nReturn JSON again.\nImportant: return exactly one placement for file_id {} and choose only from allowed_targets.",
                paper.file_id
            );
        }
    }

    Err(AppError::Validation(format!(
        "failed placement tie-break for {}: {}",
        paper.file_id, last_issue
    )))
}

fn validate_tiebreak_response(
    placements: &[PlacementDecision],
    paper: &PaperText,
    allowed_targets: &[String],
) -> Option<String> {
    if placements.len() != 1 {
        return Some("return exactly one placement".to_string());
    }
    let placement = &placements[0];
    if placement.file_id != paper.file_id {
        return Some(format!(
            "returned file_id {} but expected {}",
            placement.file_id, paper.file_id
        ));
    }
    if !allowed_targets
        .iter()
        .any(|target| target == &placement.target_rel_path)
    {
        return Some(format!(
            "target_rel_path {} is not in the embedding-ranked shortlist",
            placement.target_rel_path
        ));
    }
    None
}

fn add_embedding_candidates_to_context(
    base_context: &Value,
    candidates: &[PlacementCandidateScore],
) -> Value {
    let mut object = base_context.as_object().cloned().unwrap_or_default();
    object.insert("embedding_ranked_targets".to_string(), json!(candidates));
    Value::Object(object)
}

async fn prepare_embedding_runtime(
    papers: &[PaperText],
    allowed_targets: &[String],
    options: &PlacementEmbeddingOptions,
    verbosity: crate::terminal::Verbosity,
) -> Result<(PlacementEmbeddingRuntime, LlmUsageSummary)> {
    let db = PaperDb::open_default().map_err(map_paperdb_error)?;
    let model_id = embedding_model_id(options.provider, options.model.clone());
    let manifest = load_manifest_from_path(&options.reference_manifest_path)?;
    let reference_set = ReferenceSetInput {
        set_id: manifest.set_id.clone(),
        manifest_path: options.reference_manifest_path.clone(),
        manifest_fingerprint: manifest_fingerprint(&options.reference_manifest_path)?,
        papers: manifest
            .papers
            .iter()
            .map(reference_paper_input_from)
            .collect(),
    };
    let embedding_client = build_embedding_client(&build_embedding_config(
        options.provider,
        options.model.clone(),
        options.base_url.clone(),
        options.api_key.clone(),
    ))?;
    let adapter = PaperDbEmbeddingAdapter {
        inner: embedding_client.as_ref(),
    };

    let mut usage = LlmUsageSummary::default();
    let reference_sync = db
        .sync_reference_set(&reference_set, &adapter, &model_id, false)
        .await
        .map_err(map_paperdb_error)?;
    usage.merge(&usage_from_db_metrics(reference_sync.metrics.as_ref()));
    verbosity.stage_line(
        "placements",
        format!(
            "{} reference index for placement from {} with {} paper(s)",
            if reference_sync.skipped {
                "reused"
            } else {
                "updated"
            },
            manifest.set_id,
            reference_sync.papers_indexed
        ),
    );

    let paper_inputs = papers.iter().map(paper_input_from).collect::<Vec<_>>();
    let paper_sync = db
        .sync_embeddings(&paper_inputs, &adapter, &model_id)
        .await
        .map_err(map_paperdb_error)?;
    usage.merge(&usage_from_db_metrics(paper_sync.metrics.as_ref()));
    verbosity.stage_line(
        "placements",
        format!(
            "placement retrieval synced {} paper embedding(s), skipped {} unchanged row(s)",
            paper_sync.embeddings_upserted, paper_sync.embeddings_skipped
        ),
    );

    let paper_embeddings = papers
        .iter()
        .filter_map(|paper| {
            db.get_embedding(&paper.file_id, &model_id.provider, &model_id.model)
                .map_err(map_paperdb_error)
                .transpose()
                .map(|record| record.map(|record| (paper.file_id.clone(), record.embedding)))
        })
        .collect::<Result<HashMap<_, _>>>()?;

    let target_query_texts = allowed_targets
        .iter()
        .map(|target| target_query_text(target))
        .collect::<Vec<_>>();
    let target_embeddings_response = embedding_client
        .embed(&crate::llm::EmbeddingRequest::from_texts(
            target_query_texts.clone(),
        ))
        .await?;
    usage.record_call(&target_embeddings_response.metrics);

    let mut target_profiles = Vec::with_capacity(allowed_targets.len());
    let mut target_embeddings = HashMap::with_capacity(allowed_targets.len());

    for ((target_rel_path, query_text), query_embedding) in allowed_targets
        .iter()
        .cloned()
        .zip(target_query_texts.into_iter())
        .zip(target_embeddings_response.embeddings.into_iter())
    {
        let matches = db
            .nearest_reference_matches(&model_id, &query_embedding.values, options.reference_top_k)
            .map_err(map_paperdb_error)?;
        let strong_matches = matches
            .iter()
            .filter(|candidate| candidate.similarity >= options.min_similarity)
            .cloned()
            .collect::<Vec<_>>();
        let (source, centroid_embedding, reference_support) =
            if strong_matches.len() >= options.min_reference_support {
                (
                    PlacementTargetProfileSource::ReferenceCentroid,
                    weighted_centroid(&strong_matches)
                        .unwrap_or_else(|| query_embedding.values.clone()),
                    strong_matches
                        .iter()
                        .map(reference_support_from)
                        .collect::<Vec<_>>(),
                )
            } else {
                (
                    PlacementTargetProfileSource::TargetPathEmbedding,
                    query_embedding.values.clone(),
                    strong_matches
                        .iter()
                        .map(reference_support_from)
                        .collect::<Vec<_>>(),
                )
            };
        target_profiles.push(PlacementTargetProfile {
            target_rel_path: target_rel_path.clone(),
            query_text,
            source,
            reference_support_count: reference_support.len(),
            reference_support,
        });
        target_embeddings.insert(target_rel_path, centroid_embedding);
    }

    Ok((
        PlacementEmbeddingRuntime {
            allowed_targets: allowed_targets.to_vec(),
            target_profiles,
            target_embeddings,
            paper_embeddings,
            candidate_top_k: options.candidate_top_k,
            min_similarity: options.min_similarity,
            min_margin: options.min_margin,
        },
        usage,
    ))
}

fn rank_targets_for_paper(
    paper: &PaperText,
    runtime: &PlacementEmbeddingRuntime,
) -> Result<Vec<PlacementCandidateScore>> {
    let paper_embedding = runtime
        .paper_embeddings
        .get(&paper.file_id)
        .ok_or_else(|| {
            AppError::Validation(format!(
                "missing stored embedding for placement paper {}",
                paper.file_id
            ))
        })?;

    let mut ranking = runtime
        .allowed_targets
        .iter()
        .filter_map(|target| {
            runtime
                .target_embeddings
                .get(target)
                .map(|target_embedding| PlacementCandidateScore {
                    target_rel_path: target.clone(),
                    similarity: cosine_similarity(paper_embedding, target_embedding),
                })
        })
        .collect::<Vec<_>>();
    ranking.sort_by(|left, right| {
        right
            .similarity
            .partial_cmp(&left.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.target_rel_path.cmp(&right.target_rel_path))
    });
    Ok(ranking)
}

fn should_use_embedding_decision(
    ranking: &[PlacementCandidateScore],
    runtime: &PlacementEmbeddingRuntime,
) -> bool {
    let Some(top) = ranking.first() else {
        return false;
    };
    if ranking.len() == 1 {
        return true;
    }
    let margin = ranking
        .get(1)
        .map(|runner_up| top.similarity - runner_up.similarity)
        .unwrap_or(f32::INFINITY);
    top.similarity >= runtime.min_similarity && margin >= runtime.min_margin
}

fn top_score_and_margin(ranking: &[PlacementCandidateScore]) -> (Option<f32>, Option<f32>) {
    let Some(top) = ranking.first() else {
        return (None, None);
    };
    let margin = ranking
        .get(1)
        .map(|runner_up| top.similarity - runner_up.similarity);
    (Some(top.similarity), margin)
}

fn weighted_centroid(matches: &[ReferenceMatchRecord]) -> Option<Vec<f32>> {
    let dimensions = matches.first()?.embedding.len();
    if dimensions == 0 {
        return None;
    }

    let mut centroid = vec![0.0_f32; dimensions];
    let mut total_weight = 0.0_f32;
    for record in matches {
        if record.embedding.len() != dimensions || record.similarity <= 0.0 {
            continue;
        }
        total_weight += record.similarity;
        for (value, dimension) in record.embedding.iter().zip(centroid.iter_mut()) {
            *dimension += value * record.similarity;
        }
    }
    if total_weight <= 0.0 {
        return None;
    }
    for dimension in &mut centroid {
        *dimension /= total_weight;
    }
    Some(centroid)
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (lhs, rhs) in left.iter().zip(right.iter()) {
        dot += lhs * rhs;
        left_norm += lhs * lhs;
        right_norm += rhs * rhs;
    }
    if left_norm <= 0.0 || right_norm <= 0.0 {
        return 0.0;
    }
    dot / (left_norm.sqrt() * right_norm.sqrt())
}

fn target_query_text(target_rel_path: &str) -> String {
    if target_rel_path == "." {
        DEFAULT_ROOT_TARGET_TEXT.to_string()
    } else {
        format!("category path: {}", target_rel_path.replace('/', " / "))
    }
}

fn reference_support_from(record: &ReferenceMatchRecord) -> PlacementReferenceSupport {
    PlacementReferenceSupport {
        paper_id: record.paper_id.clone(),
        title: record.title.clone(),
        similarity: record.similarity,
    }
}

fn reference_paper_input_from(paper: &CuratedPaper) -> ReferencePaperInput {
    ReferencePaperInput {
        paper_id: paper.paper_id.clone(),
        title: paper.title.clone(),
        category: paper.category.clone(),
        subcategory: paper.subcategory.clone(),
        abstract_excerpt: paper.abstract_excerpt.clone(),
        embedding_text: format!(
            "title: {}\ncategory: {}\nsubcategory: {}\nabstract: {}",
            paper.title, paper.category, paper.subcategory, paper.abstract_excerpt
        ),
    }
}

fn manifest_fingerprint(path: &std::path::Path) -> Result<String> {
    let raw = fs::read(path)?;
    let mut hasher = DefaultHasher::new();
    raw.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
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

#[cfg(test)]
mod runtime_tests {
    use super::*;

    #[test]
    fn weighted_centroid_uses_similarity_weights() {
        let centroid = weighted_centroid(&[
            ReferenceMatchRecord {
                set_id: "demo".to_string(),
                paper_id: "p1".to_string(),
                title: "Paper One".to_string(),
                category: "AI".to_string(),
                subcategory: "Vision".to_string(),
                abstract_excerpt: String::new(),
                embedding: vec![1.0, 0.0],
                similarity: 0.75,
            },
            ReferenceMatchRecord {
                set_id: "demo".to_string(),
                paper_id: "p2".to_string(),
                title: "Paper Two".to_string(),
                category: "AI".to_string(),
                subcategory: "Vision".to_string(),
                abstract_excerpt: String::new(),
                embedding: vec![0.0, 1.0],
                similarity: 0.25,
            },
        ])
        .expect("centroid");

        assert!((centroid[0] - 0.75).abs() < 1e-6);
        assert!((centroid[1] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn embedding_decision_requires_similarity_and_margin() {
        let runtime = PlacementEmbeddingRuntime {
            allowed_targets: vec!["AI/Vision".to_string(), "AI/NLP".to_string()],
            target_profiles: Vec::new(),
            target_embeddings: HashMap::new(),
            paper_embeddings: HashMap::new(),
            candidate_top_k: 3,
            min_similarity: 0.20,
            min_margin: 0.05,
        };

        assert!(should_use_embedding_decision(
            &[
                PlacementCandidateScore {
                    target_rel_path: "AI/Vision".to_string(),
                    similarity: 0.80,
                },
                PlacementCandidateScore {
                    target_rel_path: "AI/NLP".to_string(),
                    similarity: 0.50,
                },
            ],
            &runtime,
        ));
        assert!(!should_use_embedding_decision(
            &[
                PlacementCandidateScore {
                    target_rel_path: "AI/Vision".to_string(),
                    similarity: 0.18,
                },
                PlacementCandidateScore {
                    target_rel_path: "AI/NLP".to_string(),
                    similarity: 0.05,
                },
            ],
            &runtime,
        ));
        assert!(!should_use_embedding_decision(
            &[
                PlacementCandidateScore {
                    target_rel_path: "AI/Vision".to_string(),
                    similarity: 0.30,
                },
                PlacementCandidateScore {
                    target_rel_path: "AI/NLP".to_string(),
                    similarity: 0.27,
                },
            ],
            &runtime,
        ));
    }
}
