use std::{collections::BTreeSet, fs, path::Path};

use walkdir::WalkDir;

use crate::error::{AppError, Result};

use super::OutputSnapshot;

/// Inspects the current output directory tree for placement decisions.
///
/// # Errors
/// Returns an error when the output tree cannot be read or relative paths
/// cannot be derived while scanning the directory structure.
pub fn inspect_output(output: &Path) -> Result<OutputSnapshot> {
    if !output.exists() {
        return Ok(OutputSnapshot {
            is_empty: true,
            existing_folders: vec![".".to_string()],
            tree_map: "<empty>".to_string(),
        });
    }

    let mut entries = fs::read_dir(output)?;
    let is_empty = match entries.next() {
        Some(item) => {
            let _ = item?;
            false
        }
        None => true,
    };

    let mut folders: BTreeSet<String> = BTreeSet::new();
    folders.insert(".".to_string());

    for entry in WalkDir::new(output).min_depth(1) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            let rel = entry
                .path()
                .strip_prefix(output)
                .map_err(|err| AppError::Execution(format!("strip prefix failed: {err}")))?;
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
            .map_err(|err| AppError::Execution(format!("strip prefix failed: {err}")))?;
        let depth = rel.components().count();
        let indent = "  ".repeat(depth.saturating_sub(1));
        let name = rel.to_string_lossy().replace('\\', "/");
        let suffix = if entry.file_type().is_dir() { "/" } else { "" };
        lines.push(format!("{indent}{name}{suffix}"));
    }

    Ok(lines.join("\n"))
}
