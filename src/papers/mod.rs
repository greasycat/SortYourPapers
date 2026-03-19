pub mod discovery;
pub mod extract;
pub mod preprocess;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::taxonomy::CategoryTree;

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
#[serde(from = "KeywordStageStateRepr")]
pub struct KeywordStageState {
    pub keyword_sets: Vec<KeywordSet>,
    pub preliminary_pairs: Vec<PreliminaryCategoryPair>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum KeywordStageStateRepr {
    Current {
        keyword_sets: Vec<KeywordSet>,
        #[serde(default)]
        preliminary_pairs: Vec<PreliminaryCategoryPair>,
    },
    Legacy(Vec<KeywordSet>),
}

impl From<KeywordStageStateRepr> for KeywordStageState {
    fn from(value: KeywordStageStateRepr) -> Self {
        match value {
            KeywordStageStateRepr::Current {
                keyword_sets,
                preliminary_pairs,
            } => Self {
                keyword_sets,
                preliminary_pairs,
            },
            KeywordStageStateRepr::Legacy(keyword_sets) => Self {
                keyword_sets,
                preliminary_pairs: Vec::new(),
            },
        }
    }
}

impl KeywordStageState {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.keyword_sets.len() == self.preliminary_pairs.len()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "SynthesizeCategoriesStateRepr")]
pub struct SynthesizeCategoriesState {
    pub categories: Vec<CategoryTree>,
    #[serde(default)]
    pub partial_categories: Vec<Vec<CategoryTree>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SynthesizeCategoriesStateRepr {
    Current {
        categories: Vec<CategoryTree>,
        #[serde(default)]
        partial_categories: Vec<Vec<CategoryTree>>,
    },
    Legacy(Vec<CategoryTree>),
}

impl From<SynthesizeCategoriesStateRepr> for SynthesizeCategoriesState {
    fn from(value: SynthesizeCategoriesStateRepr) -> Self {
        match value {
            SynthesizeCategoriesStateRepr::Current {
                categories,
                partial_categories,
            } => Self {
                categories,
                partial_categories,
            },
            SynthesizeCategoriesStateRepr::Legacy(categories) => Self {
                categories,
                partial_categories: Vec::new(),
            },
        }
    }
}
