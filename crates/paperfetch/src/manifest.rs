use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use syp_core::error::{AppError, Result};

use crate::{SamplingBucket, SamplingPolicy};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CuratedTestSet {
    pub id: String,
    pub description: String,
    pub source_dataset: String,
    pub selection_policy: SamplingPolicy,
    pub generated_at_ms: i64,
    #[serde(default)]
    pub papers: Vec<CuratedPaperEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CuratedPaperEntry {
    pub paper_id: String,
    pub arxiv_id: String,
    pub canonical_pdf_url: String,
    pub title: String,
    pub category: String,
    pub subcategory: String,
    pub citations: u64,
    pub date: Option<String>,
    pub abstract_excerpt: String,
    pub selection_bucket: SamplingBucket,
    pub sha256: Option<String>,
    pub byte_size: Option<u64>,
}

pub fn load_test_set(path: impl AsRef<Path>) -> Result<CuratedTestSet> {
    let raw = fs::read_to_string(path)?;
    let set: CuratedTestSet = toml::from_str(&raw)
        .map_err(|err| AppError::Validation(format!("invalid manifest: {err}")))?;
    validate_test_set(&set)?;
    Ok(set)
}

pub fn save_test_set(path: impl AsRef<Path>, set: &CuratedTestSet) -> Result<()> {
    validate_test_set(set)?;

    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let raw = toml::to_string_pretty(set)
        .map_err(|err| AppError::Execution(format!("failed to serialize manifest: {err}")))?;
    fs::write(path, raw)?;
    Ok(())
}

pub(crate) fn validate_test_set(set: &CuratedTestSet) -> Result<()> {
    if set.id.trim().is_empty() {
        return Err(AppError::Validation(
            "test set id must not be empty".to_string(),
        ));
    }
    if set.source_dataset.trim().is_empty() {
        return Err(AppError::Validation(
            "source_dataset must not be empty".to_string(),
        ));
    }

    let mut seen_paper_ids = std::collections::HashSet::new();
    let mut seen_arxiv_ids = std::collections::HashSet::new();
    for paper in &set.papers {
        if paper.paper_id.trim().is_empty() {
            return Err(AppError::Validation(
                "paper entries must include a paper_id".to_string(),
            ));
        }
        if paper.arxiv_id.trim().is_empty() {
            return Err(AppError::Validation(
                "paper entries must include an arxiv_id".to_string(),
            ));
        }
        if paper.canonical_pdf_url.trim().is_empty() {
            return Err(AppError::Validation(
                "paper entries must include a canonical_pdf_url".to_string(),
            ));
        }
        if paper.category.trim().is_empty() || paper.subcategory.trim().is_empty() {
            return Err(AppError::Validation(
                "paper entries must include both category and subcategory".to_string(),
            ));
        }
        if !seen_paper_ids.insert(paper.paper_id.as_str()) {
            return Err(AppError::Validation(format!(
                "duplicate paper_id {} in manifest",
                paper.paper_id
            )));
        }
        if !seen_arxiv_ids.insert(paper.arxiv_id.as_str()) {
            return Err(AppError::Validation(format!(
                "duplicate arxiv_id {} in manifest",
                paper.arxiv_id
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{CuratedPaperEntry, CuratedTestSet, load_test_set, save_test_set};
    use crate::{SamplingBucket, SamplingPolicy};

    #[test]
    fn manifest_round_trip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("set.toml");
        let set = CuratedTestSet {
            id: "demo".to_string(),
            description: "Demo".to_string(),
            source_dataset: "OpenMOSS-Team/SciJudgeBench".to_string(),
            selection_policy: SamplingPolicy::default(),
            generated_at_ms: 123,
            papers: vec![CuratedPaperEntry {
                paper_id: "arxiv-1234.5678".to_string(),
                arxiv_id: "1234.5678".to_string(),
                canonical_pdf_url: "https://arxiv.org/pdf/1234.5678.pdf".to_string(),
                title: "Title".to_string(),
                category: "CS".to_string(),
                subcategory: "cs.AI".to_string(),
                citations: 42,
                date: Some("2024-01-01".to_string()),
                abstract_excerpt: "Excerpt".to_string(),
                selection_bucket: SamplingBucket::Top,
                sha256: Some("abc".to_string()),
                byte_size: Some(10),
            }],
        };

        save_test_set(&path, &set).expect("save manifest");
        let loaded = load_test_set(&path).expect("load manifest");

        assert_eq!(loaded, set);
    }
}
