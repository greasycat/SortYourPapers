use super::*;

pub(super) async fn generate_placement_batch(
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

        let schema = llm_tiebreak::placement_response_schema();
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
        let ranking = embedding::rank_targets_for_paper(paper, embedding_runtime)?;
        let (top_score, margin_over_runner_up) = scoring::top_score_and_margin(&ranking);
        let top_candidates = ranking
            .iter()
            .take(embedding_runtime.candidate_top_k)
            .cloned()
            .collect::<Vec<_>>();

        if embedding::should_use_embedding_decision(&ranking, embedding_runtime) {
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

        let (placement, tie_break_usage) = llm_tiebreak::generate_tiebreak_placement(
            client,
            paper,
            base_context,
            &top_candidates,
            runtime,
        )
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
