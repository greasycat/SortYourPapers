use crate::{
    error::{AppError, Result},
    models::PaperText,
};

use super::{MAX_SEMANTIC_ATTEMPTS, MAX_TEXT_CHARS_PER_FILE, MAX_TOTAL_BATCH_TEXT_CHARS};

pub(super) fn build_category_prompt(
    aggregated_preliminary_categories: &[(String, usize)],
    category_depth: u8,
) -> Result<String> {
    Ok(format!(
        "Return JSON with schema:\n{{\"categories\":[[\"Top Level\"],[\"Top Level\",\"Subcategory\"]]}}\nRules:\n- use only the aggregated preliminary category texts below\n- each entry in `categories` must be a full category path from root to a category node\n- include parent paths before child paths\n- category depth must be <= {category_depth}\n- names must be filesystem-friendly (letters, numbers, spaces, dashes)\n- avoid duplicate category paths\n- output at least one top-level category path\n\naggregated_preliminary_categories:\n{}",
        serde_json::to_string(aggregated_preliminary_categories).map_err(AppError::from)?
    ))
}

pub(super) fn build_batch_keyword_prompt(batch: &[PaperText]) -> Result<String> {
    let per_file_limit =
        (MAX_TOTAL_BATCH_TEXT_CHARS / batch.len().max(1)).clamp(400, MAX_TEXT_CHARS_PER_FILE);

    let files = batch
        .iter()
        .map(|paper| {
            serde_json::json!({
                "file_id": paper.file_id,
                "terms": truncate_for_prompt(&paper.llm_ready_text, per_file_limit),
            })
        })
        .collect::<Vec<_>>();

    Ok(format!(
        "Return JSON with this exact schema:\n{{\"pairs\":[{{\"file_id\":\"...\",\"keywords\":[\"...\"],\"preliminary_categories_k_depth\":\"...\"}}]}}\nRules:\n- Return exactly {} pairs\n- Include every file_id exactly once\n- Keep 5 to 12 keywords for each file\n- Keywords must be specific nouns or short noun phrases\n- `preliminary_categories_k_depth` must be plain text only; it can be approximate or imperfect, but it should describe a k-depth category suggestion for the file\n- The `terms` field is a preprocessed deduplicated term list derived from the paper, not raw prose\n- No markdown\n\nfiles:\n{}",
        batch.len(),
        serde_json::to_string(&files).map_err(AppError::from)?
    ))
}

pub(super) fn format_llm_request_debug_message(
    label: &str,
    attempt: usize,
    system: &str,
    user: &str,
) -> String {
    format!(
        "{label} request attempt {attempt}/{MAX_SEMANTIC_ATTEMPTS}\nsystem:\n{system}\nuser:\n{user}"
    )
}

pub(super) fn format_batch_span(batch: &[PaperText]) -> String {
    let Some(first) = batch.first() else {
        return "<empty>".to_string();
    };
    let Some(last) = batch.last() else {
        return "<empty>".to_string();
    };
    format!(
        "{}..{} ({} files)",
        first.path.display(),
        last.path.display(),
        batch.len()
    )
}

fn truncate_for_prompt(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    input.chars().take(max_chars).collect::<String>()
}
