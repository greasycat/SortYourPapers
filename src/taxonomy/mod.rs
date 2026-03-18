mod batching;
mod keywords;
mod prompts;
mod taxonomy;
mod validation;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use crate::models::{KeywordSet, LlmUsageSummary, PreliminaryCategoryPair};

const MAX_JSON_ATTEMPTS: usize = 3;
const MAX_SEMANTIC_ATTEMPTS: usize = 3;
const MAX_TEXT_CHARS_PER_FILE: usize = 4_000;
const MAX_TOTAL_BATCH_TEXT_CHARS: usize = 60_000;
const MAX_CONCURRENT_KEYWORD_BATCH_REQUESTS: usize = 4;
const GLOBAL_TAXONOMY_LABEL: &str = "taxonomy/global";

#[derive(Debug, Deserialize)]
struct KeywordPair {
    file_id: String,
    keywords: Vec<String>,
    preliminary_categories_k_depth: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct KeywordBatchResult {
    batch_index: usize,
    keyword_sets: Vec<KeywordSet>,
    preliminary_pairs: Vec<PreliminaryCategoryPair>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct KeywordBatchProgress {
    pub(crate) completed_batches: Vec<KeywordBatchResult>,
    pub(crate) usage: LlmUsageSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TaxonomyBatchResult {
    pub(crate) batch_index: usize,
    pub(crate) input_count: usize,
    #[serde(default)]
    pub(crate) input_fingerprint: Option<String>,
    pub(crate) categories: Vec<crate::models::CategoryTree>,
    pub(crate) elapsed_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct TaxonomyBatchProgress {
    pub(crate) completed_batches: Vec<TaxonomyBatchResult>,
    pub(crate) usage: LlmUsageSummary,
}

pub use keywords::extract_keywords;
pub(crate) use keywords::extract_keywords_with_progress;
pub use taxonomy::synthesize_categories;
#[allow(unused_imports)]
pub(crate) use taxonomy::synthesize_categories_with_progress;
pub(crate) use taxonomy::{merge_category_batches, synthesize_category_batches_with_progress};
pub use validation::validate_category_depth;
