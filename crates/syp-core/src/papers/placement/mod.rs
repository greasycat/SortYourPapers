mod batching;
mod inspect;
mod prompts;
mod runtime;
mod validation;

#[cfg(test)]
mod tests;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{llm::LlmUsageSummary, papers::taxonomy::CategoryTree, terminal::Verbosity};

const MAX_JSON_ATTEMPTS: usize = 3;
const MAX_SEMANTIC_ATTEMPTS: usize = 3;
const PLACEMENT_LABEL: &str = "generate-placements";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementMode {
    #[default]
    ExistingOnly,
    AllowNew,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementDecision {
    pub file_id: String,
    pub target_rel_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSnapshot {
    pub is_empty: bool,
    pub existing_folders: Vec<String>,
    pub tree_map: String,
}

#[derive(Debug, Clone, Copy)]
pub struct PlacementOptions {
    pub batch_size: usize,
    pub batch_start_delay_ms: u64,
    pub placement_mode: PlacementMode,
    pub category_depth: u8,
    pub verbosity: Verbosity,
}

#[derive(Debug, Clone)]
struct PlacementBatchRuntime {
    categories: std::sync::Arc<Vec<CategoryTree>>,
    snapshot: std::sync::Arc<OutputSnapshot>,
    options: PlacementOptions,
    total_batches: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PlacementBatchResult {
    pub(crate) batch_index: usize,
    pub(crate) file_ids: Vec<String>,
    pub(crate) placements: Vec<PlacementDecision>,
    pub(crate) elapsed_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct PlacementBatchProgress {
    pub(crate) completed_batches: Vec<PlacementBatchResult>,
    pub(crate) usage: LlmUsageSummary,
}

pub use inspect::inspect_output;
pub use runtime::generate_placements;
pub(crate) use runtime::generate_placements_with_progress;
