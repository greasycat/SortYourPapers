use std::{
    collections::HashSet,
    path::{Component, Path},
};

use crate::{
    error::{AppError, Result},
    papers::PaperText,
};

use super::{OutputSnapshot, PlacementDecision, PlacementMode};

pub(super) fn validate_placements(
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
        .map(|paper| paper.file_id.clone())
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
                "placement path '{rel}' depth {depth} exceeds max {category_depth}"
            )));
        }

        if !snapshot.is_empty
            && placement_mode == PlacementMode::ExistingOnly
            && !existing_folder_set.contains(&rel)
        {
            return Err(AppError::Validation(format!(
                "path '{rel}' does not exist in output tree"
            )));
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
                    "target_rel_path contains illegal segment: {raw}"
                )));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }

    Ok(if normalized == "." {
        ".".to_string()
    } else {
        normalized.trim_matches('/').to_string()
    })
}

pub(super) fn path_depth(path: &str) -> usize {
    if path == "." {
        return 0;
    }
    Path::new(path)
        .components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count()
}
