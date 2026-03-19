use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use crate::{
    error::{AppError, Result},
    papers::PaperText,
    placement::PlacementDecision,
    report::{FileAction, PlanAction},
};

pub fn build_move_plan(
    output_root: &Path,
    papers: &[PaperText],
    placements: &[PlacementDecision],
) -> Result<Vec<PlanAction>> {
    let mut paper_map = HashMap::new();
    for paper in papers {
        paper_map.insert(paper.file_id.as_str(), paper.path.clone());
    }

    let mut used_destinations = HashSet::<PathBuf>::new();
    let mut actions = Vec::new();

    for placement in placements {
        let source = paper_map
            .get(placement.file_id.as_str())
            .cloned()
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "placement references missing paper {}",
                    placement.file_id
                ))
            })?;

        let file_name = source
            .file_name()
            .ok_or_else(|| AppError::Validation("source file has no filename".to_string()))?;

        let rel_dir = placement.target_rel_path.trim();
        let target_dir = if rel_dir == "." {
            output_root.to_path_buf()
        } else {
            output_root.join(rel_dir)
        };

        let tentative = target_dir.join(file_name);
        let destination = resolve_destination_conflict(&tentative, &source, &mut used_destinations);

        if source == destination {
            continue;
        }

        used_destinations.insert(destination.clone());

        actions.push(PlanAction {
            source,
            destination,
            action: FileAction::Move,
        });
    }

    Ok(actions)
}

fn resolve_destination_conflict(
    tentative: &Path,
    source: &Path,
    used_destinations: &mut HashSet<PathBuf>,
) -> PathBuf {
    if is_available_destination(tentative, source, used_destinations) {
        return tentative.to_path_buf();
    }

    let parent = tentative.parent().unwrap_or_else(|| Path::new("."));
    let file_name = tentative
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");

    let (stem, ext) = split_name(file_name);

    let mut i = 1usize;
    loop {
        let candidate_name = if ext.is_empty() {
            format!("{stem}_{i}")
        } else {
            format!("{stem}_{i}.{ext}")
        };
        let candidate = parent.join(candidate_name);

        if is_available_destination(&candidate, source, used_destinations) {
            return candidate;
        }

        i += 1;
    }
}

fn is_available_destination(
    candidate: &Path,
    source: &Path,
    used_destinations: &HashSet<PathBuf>,
) -> bool {
    if candidate == source {
        return true;
    }
    if used_destinations.contains(candidate) {
        return false;
    }
    !candidate.exists()
}

fn split_name(file_name: &str) -> (String, String) {
    match file_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => {
            (stem.to_string(), ext.to_string())
        }
        _ => (file_name.to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::Path;

    use super::resolve_destination_conflict;

    #[test]
    fn suffix_is_added_on_conflict() {
        let source = Path::new("/tmp/a.pdf");
        let tentative = Path::new("/tmp/out/a.pdf");
        let mut used = HashSet::new();
        used.insert(tentative.to_path_buf());

        let resolved = resolve_destination_conflict(tentative, source, &mut used);
        assert_eq!(resolved, Path::new("/tmp/out/a_1.pdf"));
    }
}
