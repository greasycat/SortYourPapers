use super::*;

pub(super) async fn generate_tiebreak_placement(
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

    let file_context = vec![add_embedding_candidates_to_context(base_context, top_candidates)];
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
        if let Some(issue) = validate_tiebreak_response(&response.value.placements, paper, &allowed_targets)
        {
            last_issue = issue;
        } else {
            usage.record_call(&response.metrics);
            let placement = response.value.placements.into_iter().next().ok_or_else(|| {
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

pub(super) fn placement_response_schema() -> JsonResponseSchema {
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
