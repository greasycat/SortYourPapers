use crate::{
    error::{AppError, Result},
    papers::PaperText,
    papers::taxonomy::CategoryTree,
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

pub(super) fn build_merge_category_prompt(
    batch_categories: &[Vec<CategoryTree>],
    category_depth: u8,
    subcategories_suggestion_number: usize,
    user_suggestion: Option<&str>,
) -> Result<String> {
    let category_paths = flatten_and_sort_category_paths(batch_categories);
    let suggestion_section = format_merge_suggestion_section(user_suggestion);

    Ok(format!(
        "Return JSON with schema:\n{{\"categories\":[[\"Top Level\"],[\"Top Level\",\"Subcategory\"]]}}\nRules:\n- merge the partial taxonomies below into one final taxonomy\n- use only the category paths below\n- each entry in `categories` must be a full category path from root to a category node\n- include parent paths before child paths\n- category depth must be <= {category_depth}\n- names must be filesystem-friendly (letters, numbers, spaces, dashes)\n- avoid duplicate category paths\n- output at least one top-level category path\n- try your best to keep the number of subcategories less than {subcategories_suggestion_number}\n- if a user merge suggestion is provided, treat it as optional guidance for shaping the final taxonomy\n\ncategory_paths:\n{}{}",
        serde_json::to_string(&category_paths).map_err(AppError::from)?,
        suggestion_section
    ))
}

pub(super) fn build_merge_category_plain_text_prompt(
    batch_categories: &[Vec<CategoryTree>],
    category_depth: u8,
    subcategories_suggestion_number: usize,
    user_suggestion: Option<&str>,
) -> Result<String> {
    let category_paths = flatten_and_sort_category_paths(batch_categories);
    let suggestion_section = format_merge_suggestion_section(user_suggestion);

    Ok(format!(
        "Return plain text only.\nRules:\n- merge the partial taxonomies below into one final taxonomy\n- use only the category paths below\n- return one full category path per line\n- use ` > ` between path segments\n- each line must be a full category path from root to a category node\n- include parent paths before child paths\n- category depth must be <= {category_depth}\n- names must be filesystem-friendly (letters, numbers, spaces, dashes)\n- avoid duplicate category paths\n- output at least one top-level category path\n- try your best to keep the number of subcategories less than {subcategories_suggestion_number}\n- no JSON\n- no markdown\n- if a user merge suggestion is provided, treat it as optional guidance for shaping the final taxonomy\n\ncategory_paths:\n{}{}",
        serde_json::to_string(&category_paths).map_err(AppError::from)?,
        suggestion_section
    ))
}

fn flatten_and_sort_category_paths(batch_categories: &[Vec<CategoryTree>]) -> Vec<Vec<String>> {
    let mut category_paths = batch_categories
        .iter()
        .flat_map(|categories| flatten_category_paths(categories))
        .collect::<Vec<_>>();
    category_paths.sort_by_key(|path| path.join("/"));
    category_paths
}

fn format_merge_suggestion_section(user_suggestion: Option<&str>) -> String {
    user_suggestion
        .map(str::trim)
        .filter(|suggestion| !suggestion.is_empty())
        .map(|suggestion| format!("\n\nuser_merge_suggestion:\n{suggestion}"))
        .unwrap_or_default()
}

fn flatten_category_paths(categories: &[CategoryTree]) -> Vec<Vec<String>> {
    let mut paths = Vec::new();
    for category in categories {
        collect_category_paths(category, &mut Vec::new(), &mut paths);
    }
    paths
}

fn collect_category_paths(
    category: &CategoryTree,
    prefix: &mut Vec<String>,
    paths: &mut Vec<Vec<String>>,
) {
    prefix.push(category.name.clone());
    paths.push(prefix.clone());
    for child in &category.children {
        collect_category_paths(child, prefix, paths);
    }
    prefix.pop();
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
