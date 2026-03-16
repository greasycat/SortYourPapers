use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Component, Path},
};

use serde::Deserialize;
use walkdir::WalkDir;

use crate::{
    error::{AppError, Result},
    llm::{LlmClient, call_json_with_retry},
    models::{CategoryTree, KeywordSet, PaperText, PlacementDecision, PlacementMode},
};

const MAX_JSON_ATTEMPTS: usize = 3;

#[derive(Debug, Clone)]
pub struct OutputSnapshot {
    pub is_empty: bool,
    pub existing_folders: Vec<String>,
    pub tree_map: String,
}

#[derive(Debug, Deserialize)]
struct PlacementResponse {
    placements: Vec<PlacementDecision>,
}

pub fn inspect_output(output: &Path) -> Result<OutputSnapshot> {
    if !output.exists() {
        return Ok(OutputSnapshot {
            is_empty: true,
            existing_folders: vec![".".to_string()],
            tree_map: "<empty>".to_string(),
        });
    }

    let mut is_empty = true;
    for item in fs::read_dir(output)? {
        let _ = item?;
        is_empty = false;
        break;
    }

    let mut folders: BTreeSet<String> = BTreeSet::new();
    folders.insert(".".to_string());

    for entry in WalkDir::new(output).min_depth(1) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            let rel = entry
                .path()
                .strip_prefix(output)
                .map_err(|e| AppError::Execution(format!("strip prefix failed: {e}")))?;
            folders.insert(rel.to_string_lossy().replace('\\', "/"));
        }
    }

    let tree_map = build_tree_map(output)?;

    Ok(OutputSnapshot {
        is_empty,
        existing_folders: folders.into_iter().collect(),
        tree_map,
    })
}

pub async fn generate_placements(
    client: &dyn LlmClient,
    papers: &[PaperText],
    keyword_sets: &[KeywordSet],
    categories: &[CategoryTree],
    snapshot: &OutputSnapshot,
    placement_mode: PlacementMode,
    category_depth: u8,
) -> Result<Vec<PlacementDecision>> {
    if papers.is_empty() {
        return Ok(Vec::new());
    }

    let keyword_map: HashMap<&str, &[String]> = keyword_sets
        .iter()
        .map(|k| (k.file_id.as_str(), k.keywords.as_slice()))
        .collect();

    let file_context = papers
        .iter()
        .map(|paper| {
            let file_name = paper
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown.pdf");
            let keywords = keyword_map
                .get(paper.file_id.as_str())
                .copied()
                .unwrap_or(&[]);
            serde_json::json!({
                "file_id": paper.file_id,
                "file_name": file_name,
                "path": paper.path.to_string_lossy(),
                "keywords": keywords,
            })
        })
        .collect::<Vec<_>>();

    let system = "You assign PDFs to category folders. Return strict JSON only.";
    let user = if snapshot.is_empty {
        format!(
            "Return JSON with schema:\n{{\"placements\":[{{\"file_id\":\"...\",\"target_rel_path\":\"...\",\"confidence\":0.0,\"rationale\":\"...\"}}]}}\nRules:\n- exactly one placement per file\n- target_rel_path must be a relative directory path (no file name)\n- max depth for target_rel_path is {category_depth}\n- use taxonomy context below\n- no markdown\n\ncategories:\n{}\n\nfiles:\n{}",
            serde_json::to_string_pretty(categories).map_err(AppError::from)?,
            serde_json::to_string_pretty(&file_context).map_err(AppError::from)?,
        )
    } else {
        format!(
            "Return JSON with schema:\n{{\"placements\":[{{\"file_id\":\"...\",\"target_rel_path\":\"...\",\"confidence\":0.0,\"rationale\":\"...\"}}]}}\nRules:\n- exactly one placement per file\n- target_rel_path must be a relative directory path (no file name)\n- max depth for target_rel_path is {category_depth}\n- placement_mode is {placement_mode:?}\n- if placement_mode is ExistingOnly, choose only from existing_folders\n- if placement_mode is AllowNew, you may use existing_folders or create new paths up to max depth\n- no markdown\n\nexisting_folders:\n{}\n\ncurrent_tree_map:\n{}\n\ncategory_hints:\n{}\n\nfiles:\n{}",
            serde_json::to_string_pretty(&snapshot.existing_folders).map_err(AppError::from)?,
            snapshot.tree_map,
            serde_json::to_string_pretty(categories).map_err(AppError::from)?,
            serde_json::to_string_pretty(&file_context).map_err(AppError::from)?,
        )
    };

    let response: PlacementResponse =
        call_json_with_retry(client, system, &user, MAX_JSON_ATTEMPTS).await?;

    validate_placements(
        &response.placements,
        papers,
        snapshot,
        placement_mode,
        category_depth,
    )?;

    Ok(response.placements)
}

fn validate_placements(
    placements: &[PlacementDecision],
    papers: &[PaperText],
    snapshot: &OutputSnapshot,
    placement_mode: PlacementMode,
    category_depth: u8,
) -> Result<()> {
    if placements.len() != papers.len() {
        return Err(AppError::Validation(format!(
            "placements count mismatch: expected {}, got {}",
            papers.len(),
            placements.len()
        )));
    }

    let expected_ids = papers
        .iter()
        .map(|p| p.file_id.clone())
        .collect::<HashSet<_>>();
    let mut seen_ids = HashSet::new();
    let existing_folder_set = snapshot
        .existing_folders
        .iter()
        .cloned()
        .collect::<HashSet<_>>();

    for placement in placements {
        if !expected_ids.contains(&placement.file_id) {
            return Err(AppError::Validation(format!(
                "placement references unknown file_id {}",
                placement.file_id
            )));
        }
        if !seen_ids.insert(placement.file_id.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate placement for file_id {}",
                placement.file_id
            )));
        }

        let rel = normalize_rel_dir(&placement.target_rel_path)?;
        let depth = path_depth(&rel);
        if depth > usize::from(category_depth) {
            return Err(AppError::Validation(format!(
                "placement path '{}' depth {} exceeds max {}",
                rel, depth, category_depth
            )));
        }

        if !snapshot.is_empty && placement_mode == PlacementMode::ExistingOnly {
            if !existing_folder_set.contains(&rel) {
                return Err(AppError::Validation(format!(
                    "path '{}' does not exist in output tree",
                    rel
                )));
            }
        }
    }

    Ok(())
}

fn normalize_rel_dir(raw: &str) -> Result<String> {
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty() {
        return Err(AppError::Validation(
            "target_rel_path cannot be empty".to_string(),
        ));
    }

    let path = Path::new(&normalized);
    if path.is_absolute() {
        return Err(AppError::Validation(
            "target_rel_path must be relative".to_string(),
        ));
    }

    for component in path.components() {
        match component {
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(AppError::Validation(format!(
                    "target_rel_path contains illegal segment: {}",
                    raw
                )));
            }
            _ => {}
        }
    }

    Ok(if normalized == "." {
        ".".to_string()
    } else {
        normalized.trim_matches('/').to_string()
    })
}

fn path_depth(path: &str) -> usize {
    if path == "." {
        return 0;
    }
    Path::new(path)
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count()
}

fn build_tree_map(root: &Path) -> Result<String> {
    if !root.exists() {
        return Ok("<missing>".to_string());
    }

    let mut lines = vec![".".to_string()];

    for entry in WalkDir::new(root).min_depth(1) {
        let entry = entry?;
        let rel = entry
            .path()
            .strip_prefix(root)
            .map_err(|e| AppError::Execution(format!("strip prefix failed: {e}")))?;
        let depth = rel.components().count();
        let indent = "  ".repeat(depth.saturating_sub(1));
        let name = rel.to_string_lossy().replace('\\', "/");
        let suffix = if entry.file_type().is_dir() { "/" } else { "" };
        lines.push(format!("{indent}{name}{suffix}"));
    }

    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::{OutputSnapshot, path_depth, validate_placements};
    use crate::models::{PaperText, PlacementDecision, PlacementMode};
    use std::path::PathBuf;

    #[test]
    fn depth_for_root_is_zero() {
        assert_eq!(path_depth("."), 0);
        assert_eq!(path_depth("a/b"), 2);
    }

    #[test]
    fn existing_only_rejects_unknown_folder() {
        let papers = vec![PaperText {
            file_id: "f1".to_string(),
            path: PathBuf::from("/tmp/p1.pdf"),
            extracted_text: "x".to_string(),
            llm_ready_text: "x".to_string(),
            pages_read: 1,
        }];
        let placements = vec![PlacementDecision {
            file_id: "f1".to_string(),
            target_rel_path: "new-folder".to_string(),
            rationale: None,
            confidence: Some(0.8),
        }];
        let snapshot = OutputSnapshot {
            is_empty: false,
            existing_folders: vec![".".to_string(), "existing".to_string()],
            tree_map: ".".to_string(),
        };

        let result = validate_placements(
            &placements,
            &papers,
            &snapshot,
            PlacementMode::ExistingOnly,
            2,
        );
        assert!(result.is_err());
    }
}
