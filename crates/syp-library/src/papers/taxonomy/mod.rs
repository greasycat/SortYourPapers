mod batching;
mod keywords;
mod prompts;
mod reference;
mod taxonomy;
mod validation;

#[cfg(test)]
mod tests;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{
    llm::LlmUsageSummary,
    papers::{KeywordSet, PreliminaryCategoryPair},
};

const MAX_JSON_ATTEMPTS: usize = 3;
const MAX_SEMANTIC_ATTEMPTS: usize = 3;
const MAX_TEXT_CHARS_PER_FILE: usize = 4_000;
const MAX_TOTAL_BATCH_TEXT_CHARS: usize = 60_000;
const MAX_CONCURRENT_KEYWORD_BATCH_REQUESTS: usize = 4;
const GLOBAL_TAXONOMY_LABEL: &str = "taxonomy/global";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TaxonomyMode {
    Global,
    #[default]
    BatchMerge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TaxonomyAssistance {
    #[default]
    LlmOnly,
    EmbeddingGuided,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryTree {
    pub name: String,
    #[serde(default)]
    pub children: Vec<CategoryTree>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReferenceLabelScore {
    pub label: String,
    pub weight: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReferenceExemplar {
    pub paper_id: String,
    pub title: String,
    pub category: String,
    pub subcategory: String,
    pub similarity: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TaxonomyReferenceEvidence {
    #[serde(default)]
    pub set_id: String,
    pub query_paper_count: usize,
    pub top_k_per_paper: usize,
    #[serde(default)]
    pub top_categories: Vec<ReferenceLabelScore>,
    #[serde(default)]
    pub top_subcategory_tokens: Vec<ReferenceLabelScore>,
    #[serde(default)]
    pub exemplar_matches: Vec<ReferenceExemplar>,
}

#[derive(Debug, Deserialize)]
struct KeywordPair {
    file_id: String,
    keywords: Vec<String>,
    preliminary_categories_k_depth: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordBatchResult {
    pub batch_index: usize,
    pub keyword_sets: Vec<KeywordSet>,
    pub preliminary_pairs: Vec<PreliminaryCategoryPair>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeywordBatchProgress {
    pub completed_batches: Vec<KeywordBatchResult>,
    pub usage: LlmUsageSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxonomyBatchResult {
    pub batch_index: usize,
    pub input_count: usize,
    #[serde(default)]
    pub input_fingerprint: Option<String>,
    pub categories: Vec<CategoryTree>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaxonomyBatchProgress {
    pub completed_batches: Vec<TaxonomyBatchResult>,
    pub usage: LlmUsageSummary,
}

pub use keywords::extract_keywords;
pub use keywords::extract_keywords_with_progress;
pub use reference::{
    ReferenceEmbeddingOptions, ReferenceEvidenceOptions, collect_reference_evidence,
    index_reference_manifest,
};
pub use taxonomy::synthesize_categories;
#[allow(unused_imports)]
pub use taxonomy::synthesize_categories_with_progress;
pub use taxonomy::{merge_category_batches, synthesize_category_batches_with_progress};
pub use validation::validate_category_depth;
