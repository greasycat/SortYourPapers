pub mod discovery;
pub mod extract;
pub mod fs_ops;
pub mod placement;
pub mod preprocess;
pub mod taxonomy;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::papers::taxonomy::{CategoryTree, TaxonomyReferenceEvidence};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfCandidate {
    pub path: PathBuf,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperText {
    pub file_id: String,
    pub path: PathBuf,
    pub extracted_text: String,
    pub llm_ready_text: String,
    pub pages_read: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordSet {
    pub file_id: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreliminaryCategoryPair {
    pub file_id: String,
    pub preliminary_categories_k_depth: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordStageState {
    pub keyword_sets: Vec<KeywordSet>,
    #[serde(default)]
    pub preliminary_pairs: Vec<PreliminaryCategoryPair>,
}

impl KeywordStageState {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.keyword_sets.len() == self.preliminary_pairs.len()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesizeCategoriesState {
    pub categories: Vec<CategoryTree>,
    #[serde(default)]
    pub partial_categories: Vec<Vec<CategoryTree>>,
    #[serde(default)]
    pub reference_evidence: Option<TaxonomyReferenceEvidence>,
}
