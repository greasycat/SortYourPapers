use crate::{
    llm::LlmProvider,
    papers::{placement::PlacementAssistance, taxonomy::TaxonomyAssistance},
};

pub const DEFAULT_INPUT: &str = ".";
pub const DEFAULT_OUTPUT: &str = "./sorted";
pub const DEFAULT_MAX_FILE_SIZE_MB: u64 = 16;
pub const DEFAULT_PAGE_CUTOFF: u8 = 1;
pub const DEFAULT_PDF_EXTRACT_WORKERS: usize = 8;
pub const DEFAULT_CATEGORY_DEPTH: u8 = 2;
pub const DEFAULT_KEYWORD_BATCH_SIZE: usize = 20;
pub const DEFAULT_BATCH_START_DELAY_MS: u64 = 100;
pub const DEFAULT_TAXONOMY_BATCH_SIZE: usize = 4;
pub const DEFAULT_PLACEMENT_BATCH_SIZE: usize = 10;
pub const DEFAULT_PLACEMENT_REFERENCE_TOP_K: usize = 5;
pub const DEFAULT_PLACEMENT_CANDIDATE_TOP_K: usize = 3;
pub const DEFAULT_PLACEMENT_MIN_SIMILARITY: f32 = 0.20;
pub const DEFAULT_PLACEMENT_MIN_MARGIN: f32 = 0.05;
pub const DEFAULT_PLACEMENT_MIN_REFERENCE_SUPPORT: usize = 2;
pub const DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER: usize = 5;
pub const DEFAULT_REFERENCE_TOP_K: usize = 5;
pub const DEFAULT_RECURSIVE: bool = false;
pub const DEFAULT_REBUILD: bool = false;
pub const DEFAULT_USE_CURRENT_FOLDER_TREE: bool = false;
pub const DEFAULT_LLM_PROVIDER: LlmProvider = LlmProvider::Gemini;
pub const DEFAULT_LLM_MODEL: &str = "gemini-3-flash-preview";
pub const DEFAULT_TAXONOMY_ASSISTANCE: TaxonomyAssistance = TaxonomyAssistance::LlmOnly;
pub const DEFAULT_PLACEMENT_ASSISTANCE: PlacementAssistance = PlacementAssistance::LlmOnly;
pub const DEFAULT_REFERENCE_MANIFEST_PATH: &str = "assets/testsets/scijudgebench-diverse.toml";
pub const DEFAULT_OPENAI_EMBEDDING_MODEL: &str = "text-embedding-3-small";
pub const DEFAULT_GEMINI_EMBEDDING_MODEL: &str = "gemini-embedding-2-preview";
