use std::collections::{BTreeSet, HashMap, HashSet};

use crate::{
    error::{AppError, Result},
    papers::taxonomy::CategoryTree,
    papers::{KeywordSet, PaperText, PreliminaryCategoryPair},
};

use super::KeywordPair;

/// Validates that all taxonomy branches stay within the configured depth limit.
///
/// # Errors
/// Returns an error when any category subtree exceeds `max_depth`.
pub fn validate_category_depth(categories: &[CategoryTree], max_depth: u8) -> Result<()> {
    for category in categories {
        let depth = tree_depth(category);
        if depth > usize::from(max_depth) {
            return Err(AppError::Validation(format!(
                "category '{}' depth {} exceeds allowed {}",
                category.name, depth, max_depth
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_category_names(categories: &[CategoryTree]) -> Result<()> {
    let mut sibling_names = HashSet::new();
    for cat in categories {
        let normalized = normalize_folder_name(&cat.name);
        if normalized.is_empty() {
            return Err(AppError::Validation(
                "category names cannot be empty".to_string(),
            ));
        }
        if !sibling_names.insert(normalized.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate sibling category name '{}'",
                cat.name
            )));
        }
        validate_category_names(&cat.children)?;
    }
    Ok(())
}

pub(super) fn aggregate_preliminary_categories(
    preliminary_pairs: &[PreliminaryCategoryPair],
) -> Vec<(String, usize)> {
    let mut counts = HashMap::<String, usize>::new();
    for pair in preliminary_pairs {
        *counts
            .entry(pair.preliminary_categories_k_depth.clone())
            .or_default() += 1;
    }

    counts
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn validate_keyword_batch_response(
    pairs: &[KeywordPair],
    batch: &[PaperText],
) -> Result<(Vec<KeywordSet>, Vec<PreliminaryCategoryPair>)> {
    if pairs.len() != batch.len() {
        return Err(AppError::Validation(format!(
            "pair count mismatch: expected {}, got {}",
            batch.len(),
            pairs.len()
        )));
    }

    let expected = batch
        .iter()
        .map(|paper| paper.file_id.as_str())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut keyword_map = HashMap::<String, Vec<String>>::new();
    let mut preliminary_map = HashMap::<String, String>::new();

    for pair in pairs {
        if !expected.contains(pair.file_id.as_str()) {
            return Err(AppError::Validation(format!(
                "response contains unknown file_id '{}'",
                pair.file_id
            )));
        }
        if !seen.insert(pair.file_id.as_str()) {
            return Err(AppError::Validation(format!(
                "duplicate file_id '{}'",
                pair.file_id
            )));
        }

        let mut deduped = Vec::new();
        let mut seen_keywords = HashSet::new();
        for keyword in &pair.keywords {
            let normalized = normalize_keyword(keyword);
            if !normalized.is_empty() && seen_keywords.insert(normalized.clone()) {
                deduped.push(normalized);
            }
        }

        if deduped.is_empty() {
            return Err(AppError::Validation(format!(
                "keywords for file_id '{}' are empty after normalization",
                pair.file_id
            )));
        }

        keyword_map.insert(pair.file_id.clone(), deduped);
        preliminary_map.insert(
            pair.file_id.clone(),
            pair.preliminary_categories_k_depth.clone(),
        );
    }

    let mut keyword_sets = Vec::with_capacity(batch.len());
    let mut preliminary_pairs = Vec::with_capacity(batch.len());
    for paper in batch {
        let Some(keywords) = keyword_map.remove(&paper.file_id) else {
            return Err(AppError::Validation(format!(
                "missing keywords for expected file_id '{}'",
                paper.file_id
            )));
        };
        let preliminary_categories_k_depth =
            preliminary_map.remove(&paper.file_id).ok_or_else(|| {
                AppError::Validation(format!(
                    "missing preliminary categories for expected file_id '{}'",
                    paper.file_id
                ))
            })?;
        keyword_sets.push(KeywordSet {
            file_id: paper.file_id.clone(),
            keywords,
        });
        preliminary_pairs.push(PreliminaryCategoryPair {
            file_id: paper.file_id.clone(),
            preliminary_categories_k_depth,
        });
    }

    Ok((keyword_sets, preliminary_pairs))
}

fn tree_depth(node: &CategoryTree) -> usize {
    if node.children.is_empty() {
        1
    } else {
        1 + node.children.iter().map(tree_depth).max().unwrap_or(0)
    }
}

fn normalize_keyword(raw: &str) -> String {
    raw.trim().replace('\n', " ")
}

fn normalize_folder_name(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '-')
        .collect::<String>()
        .trim()
        .to_string()
}
