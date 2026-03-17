use std::collections::HashSet;

use serde::Deserialize;

use crate::{
    error::{AppError, Result},
    llm::{JsonResponseSchema, LlmClient, call_json_with_retry},
    logging::{ProgressTracker, Verbosity},
    models::{CategoryTree, LlmUsageSummary, PreliminaryCategoryPair},
};

use super::{
    GLOBAL_TAXONOMY_LABEL, MAX_JSON_ATTEMPTS, MAX_SEMANTIC_ATTEMPTS,
    prompts::{build_category_prompt, format_llm_request_debug_message},
    validation::{
        aggregate_preliminary_categories, validate_category_depth, validate_category_names,
    },
};

#[derive(Debug, Deserialize)]
struct CategoryResponse {
    categories: Vec<Vec<String>>,
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
    verbosity: Verbosity,
) -> Result<(Vec<CategoryTree>, LlmUsageSummary)> {
    if preliminary_pairs.is_empty() {
        return Ok((Vec::new(), LlmUsageSummary::default()));
    }

    let aggregated_preliminary_categories = aggregate_preliminary_categories(preliminary_pairs);
    verbosity.stage_line(
        "taxonomy",
        format!(
            "synthesizing categories from {} aggregated preliminary categor(ies) with max depth {}",
            aggregated_preliminary_categories.len(),
            category_depth
        ),
    );
    let base_user = build_category_prompt(&aggregated_preliminary_categories, category_depth)?;
    let mut progress = ProgressTracker::new(verbosity, 1, "taxonomy", true);
    let (categories, usage) = request_validated_categories(
        client,
        "You design hierarchical folder taxonomies for academic PDFs. Return strict JSON only.",
        &base_user,
        category_depth,
        verbosity,
        GLOBAL_TAXONOMY_LABEL,
    )
    .await?;
    progress.inc(1);
    progress.finish();
    Ok((categories, usage))
}

async fn request_validated_categories(
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

        let categories = match rebuild_category_tree(&response.value.categories) {
            Ok(categories) => categories,
            Err(err) => {
                last_issue = err.to_string();
                if attempt < MAX_SEMANTIC_ATTEMPTS {
                    response.metrics.semantic_retry_count += 1;
                }
                usage.record_call(&response.metrics);
                if attempt < MAX_SEMANTIC_ATTEMPTS {
                    user = format!(
                        "{base_user}\n\nYour previous response failed validation with this error: {last_issue}.\nReturn corrected JSON that satisfies all rules."
                    );
                }
                continue;
            }
        };

        if categories.is_empty() {
            last_issue = "category synthesis returned no categories".to_string();
        } else if let Err(err) = validate_category_depth(&categories, category_depth) {
            last_issue = err.to_string();
        } else if let Err(err) = validate_category_names(&categories) {
            last_issue = err.to_string();
        } else {
            usage.record_call(&response.metrics);
            return Ok((categories, usage));
        }

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
