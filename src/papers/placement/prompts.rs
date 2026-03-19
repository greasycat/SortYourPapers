use std::collections::{BTreeSet, HashMap};

use serde_json::Value;

use crate::{
    error::{AppError, Result},
    papers::PaperText,
    papers::taxonomy::CategoryTree,
};

use super::{OutputSnapshot, PLACEMENT_LABEL, PlacementMode};

pub(super) fn format_placement_request_debug_message(system: &str, user: &str) -> String {
    format!("{PLACEMENT_LABEL} request\nsystem:\n{system}\nuser:\n{user}")
}

pub(super) fn build_file_context(
    papers: &[PaperText],
    keyword_map: &HashMap<&str, &[String]>,
    preliminary_map: &HashMap<&str, &str>,
) -> Vec<Value> {
    papers
        .iter()
        .map(|paper| {
            let file_name = paper
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown.pdf");
            let keywords = keyword_map
                .get(paper.file_id.as_str())
                .copied()
                .unwrap_or(&[]);
            let preliminary_categories_k_depth = preliminary_map
                .get(paper.file_id.as_str())
                .copied()
                .unwrap_or("");
            serde_json::json!({
                "file_id": paper.file_id,
                "file_name": file_name,
                "keywords": keywords,
                "preliminary_categories_k_depth": preliminary_categories_k_depth,
            })
        })
        .collect()
}

pub(super) fn build_placement_prompt(
    file_context: &[Value],
    allowed_targets: &[String],
) -> Result<String> {
    Ok(format!(
        "Return JSON with schema:\n{{\"placements\":[{{\"file_id\":\"...\",\"target_rel_path\":\"...\"}}]}}\nRules:\n- exactly one placement per file\n- choose the best final subcategory using each file's keywords and preliminary_categories_k_depth text\n- target_rel_path must be one of allowed_targets\n- target_rel_path must be a relative directory path (no file name)\n- no markdown\n\nallowed_targets:\n{}\n\nfiles:\n{}",
        serde_json::to_string(allowed_targets).map_err(AppError::from)?,
        serde_json::to_string(file_context).map_err(AppError::from)?,
    ))
}

pub(super) fn build_allowed_targets(
    categories: &[CategoryTree],
    snapshot: &OutputSnapshot,
    placement_mode: PlacementMode,
    category_depth: u8,
) -> Vec<String> {
    let mut allowed = snapshot
        .existing_folders
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    if snapshot.is_empty || placement_mode != PlacementMode::ExistingOnly {
        for category in categories {
            collect_category_paths(category, "", 1, category_depth, &mut allowed);
        }
    }

    allowed.into_iter().collect()
}

pub(super) fn format_paper_batch_span(papers: &[PaperText]) -> String {
    let Some(first) = papers.first() else {
        return "empty batch".to_string();
    };
    let Some(last) = papers.last() else {
        return "empty batch".to_string();
    };
    format!(
        "file_ids {}..{} ({} file(s))",
        first.file_id,
        last.file_id,
        papers.len()
    )
}

fn collect_category_paths(
    category: &CategoryTree,
    prefix: &str,
    current_depth: u8,
    max_depth: u8,
    allowed: &mut BTreeSet<String>,
) {
    if current_depth > max_depth {
        return;
    }

    let path = if prefix.is_empty() {
        category.name.clone()
    } else {
        format!("{prefix}/{}", category.name)
    };
    allowed.insert(path.clone());

    for child in &category.children {
        collect_category_paths(child, &path, current_depth + 1, max_depth, allowed);
    }
}
