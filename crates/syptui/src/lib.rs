pub type CliArgs = syp_core::inputs::RunOverrides;
pub type ExtractTextArgs = syp_core::inputs::ExtractTextRequest;

pub mod cli {
    pub use syp_core::defaults::{
        DEFAULT_CATEGORY_DEPTH, DEFAULT_INPUT, DEFAULT_KEYWORD_BATCH_SIZE, DEFAULT_LLM_MODEL,
        DEFAULT_LLM_PROVIDER, DEFAULT_MAX_FILE_SIZE_MB, DEFAULT_OUTPUT, DEFAULT_PAGE_CUTOFF,
        DEFAULT_PDF_EXTRACT_WORKERS, DEFAULT_PLACEMENT_BATCH_SIZE,
        DEFAULT_SUBCATEGORIES_SUGGESTION_NUMBER, DEFAULT_TAXONOMY_BATCH_SIZE,
    };
}

pub mod prefs;
pub mod tui;

pub use syp_core::{app, config, error, llm, papers, report, session, terminal};
pub use syp_core::{rerun_run, resume_run, run};
