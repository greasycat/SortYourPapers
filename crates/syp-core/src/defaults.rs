use crate::llm::LlmProvider;

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
pub const DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER: usize = 5;
pub const DEFAULT_RECURSIVE: bool = false;
pub const DEFAULT_REBUILD: bool = false;
pub const DEFAULT_USE_CURRENT_FOLDER_TREE: bool = false;
pub const DEFAULT_LLM_PROVIDER: LlmProvider = LlmProvider::Gemini;
pub const DEFAULT_LLM_MODEL: &str = "gemini-3-flash-preview";
