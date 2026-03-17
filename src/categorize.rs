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

pub use keywords::extract_keywords;
pub(crate) use keywords::extract_keywords_with_progress;
pub use taxonomy::synthesize_categories;
pub use validation::validate_category_depth;
