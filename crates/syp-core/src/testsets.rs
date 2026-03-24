use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    config,
    error::{AppError, Result},
};

const FILES_DIR: &str = "files";
const MANIFEST_JSON_FILE: &str = "manifest.json";
const MANIFEST_TOML_FILE: &str = "manifest.toml";
const STATE_FILE: &str = "state.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamplingPolicy {
    pub top_n_per_category: u32,
    pub bottom_n_per_category: u32,
    pub random_n_per_category: u32,
    pub random_seed: u64,
    pub per_subcategory_cap: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionBucket {
    Top,
    Bottom,
    Random,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CuratedPaper {
    pub paper_id: String,
    pub arxiv_id: String,
    pub title: String,
    pub category: String,
    pub subcategory: String,
    pub citations: i64,
    pub date: Option<String>,
    pub abstract_excerpt: String,
    pub selection_bucket: SelectionBucket,
    pub paper_url: String,
    pub pdf_url: String,
    #[serde(default)]
    pub source_splits: Vec<String>,
    pub sha256: Option<String>,
    pub byte_size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CuratedTestSet {
    #[serde(rename = "id")]
    pub set_id: String,
    pub description: String,
    pub source_dataset: String,
    pub selection_policy: SamplingPolicy,
    pub generated_at_ms: i64,
    #[serde(default)]
    pub papers: Vec<CuratedPaper>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializedStateEntry {
    pub arxiv_id: String,
    pub source_url: String,
    pub sha256: String,
    pub byte_size: u64,
    pub verified_at_ms: i64,
}

pub type MaterializedState = BTreeMap<String, MaterializedStateEntry>;

pub fn shared_cache_root() -> Result<PathBuf> {
    config::shared_testset_cache_dir()
}

pub fn set_cache_dir(set_id: &str) -> Result<PathBuf> {
    Ok(shared_cache_root()?.join(set_id))
}

pub fn files_dir(set_id: &str) -> Result<PathBuf> {
    Ok(set_cache_dir(set_id)?.join(FILES_DIR))
}

pub fn manifest_json_path(set_id: &str) -> Result<PathBuf> {
    Ok(set_cache_dir(set_id)?.join(MANIFEST_JSON_FILE))
}

pub fn manifest_toml_path(set_id: &str) -> Result<PathBuf> {
    Ok(set_cache_dir(set_id)?.join(MANIFEST_TOML_FILE))
}

pub fn state_path(set_id: &str) -> Result<PathBuf> {
    Ok(set_cache_dir(set_id)?.join(STATE_FILE))
}

pub fn materialized_pdf_path(set_id: &str, paper_id: &str) -> Result<PathBuf> {
    Ok(files_dir(set_id)?.join(format!("{paper_id}.pdf")))
}

pub fn load_manifest(set_id: &str) -> Result<CuratedTestSet> {
    load_manifest_from_path(&manifest_json_path(set_id)?)
}

pub fn load_manifest_from_path(path: &Path) -> Result<CuratedTestSet> {
    load_manifest_file_from_path(path)
}

pub fn load_state(set_id: &str) -> Result<MaterializedState> {
    load_state_from_path(&state_path(set_id)?)
}

fn load_manifest_file_from_path(path: &Path) -> Result<CuratedTestSet> {
    let raw = fs::read_to_string(path)?;
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("toml") => toml::from_str(&raw).map_err(AppError::from),
        _ => serde_json::from_str(&raw).map_err(AppError::from),
    }
}

fn load_state_from_path(path: &Path) -> Result<MaterializedState> {
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw).map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        CuratedTestSet, MaterializedState, load_manifest_from_path, load_state_from_path,
        materialized_pdf_path,
    };

    #[test]
    fn loads_manifest_json() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("manifest.json");
        fs::write(
            &path,
            r#"{
  "id": "demo",
  "description": "Demo set",
  "source_dataset": "OpenMOSS-Team/SciJudgeBench",
  "selection_policy": {
    "top_n_per_category": 5,
    "bottom_n_per_category": 5,
    "random_n_per_category": 5,
    "random_seed": 1511510650,
    "per_subcategory_cap": 2
  },
  "generated_at_ms": 1,
  "papers": [
    {
      "paper_id": "arxiv-1234.5678",
      "arxiv_id": "1234.5678",
      "title": "Example",
      "category": "CS",
      "subcategory": "cs.AI",
      "citations": 10,
      "date": "2024-01-01",
      "abstract_excerpt": "Excerpt",
      "selection_bucket": "top",
      "paper_url": "https://arxiv.org/abs/1234.5678",
      "pdf_url": "https://arxiv.org/pdf/1234.5678.pdf",
      "source_splits": ["train"],
      "sha256": "abc",
      "byte_size": 12
    }
  ]
}"#,
        )
        .expect("write manifest");

        let manifest: CuratedTestSet = load_manifest_from_path(&path).expect("load manifest");
        assert_eq!(manifest.set_id, "demo");
        assert_eq!(manifest.papers.len(), 1);
        assert_eq!(manifest.papers[0].paper_id, "arxiv-1234.5678");
        assert_eq!(manifest.papers[0].byte_size, Some(12));
    }

    #[test]
    fn loads_manifest_toml() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("manifest.toml");
        fs::write(
            &path,
            r#"
id = "demo"
description = "Demo set"
source_dataset = "OpenMOSS-Team/SciJudgeBench"
generated_at_ms = 1

[selection_policy]
top_n_per_category = 5
bottom_n_per_category = 5
random_n_per_category = 5
random_seed = 1511510650
per_subcategory_cap = 2

[[papers]]
paper_id = "arxiv-1234.5678"
arxiv_id = "1234.5678"
title = "Example"
category = "CS"
subcategory = "cs.AI"
citations = 10
date = "2024-01-01"
abstract_excerpt = "Excerpt"
selection_bucket = "top"
paper_url = "https://arxiv.org/abs/1234.5678"
pdf_url = "https://arxiv.org/pdf/1234.5678.pdf"
source_splits = ["train"]
"#,
        )
        .expect("write manifest");

        let manifest: CuratedTestSet = load_manifest_from_path(&path).expect("load manifest");
        assert_eq!(manifest.set_id, "demo");
        assert_eq!(manifest.papers.len(), 1);
        assert_eq!(manifest.papers[0].subcategory, "cs.AI");
    }

    #[test]
    fn loads_state_json() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        fs::write(
            &path,
            r#"{
  "arxiv-1234.5678": {
    "arxiv_id": "1234.5678",
    "source_url": "https://arxiv.org/pdf/1234.5678.pdf",
    "sha256": "abc",
    "byte_size": 12,
    "verified_at_ms": 1
  }
}"#,
        )
        .expect("write state");

        let state: MaterializedState = load_state_from_path(&path).expect("load state");
        assert_eq!(state.len(), 1);
        assert_eq!(state.get("arxiv-1234.5678").expect("entry").sha256, "abc");
    }

    #[test]
    fn builds_materialized_pdf_path() {
        let path = materialized_pdf_path("demo", "arxiv-1234.5678").expect("pdf path");
        assert!(path.ends_with("demo/files/arxiv-1234.5678.pdf"));
    }
}
