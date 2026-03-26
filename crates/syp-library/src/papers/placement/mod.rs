mod batching;
mod inspect;
mod prompts;
mod runtime;
mod validation;

#[cfg(test)]
mod tests;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use std::path::PathBuf;

use crate::{
    llm::{LlmProvider, LlmUsageSummary},
    papers::taxonomy::CategoryTree,
    terminal::Verbosity,
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementAssistance {
    #[default]
    LlmOnly,
    EmbeddingPrimary,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementDecisionSource {
    LlmOnly,
    Embedding,
    LlmTiebreak,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementTargetProfileSource {
    ReferenceCentroid,
    TargetPathEmbedding,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementCandidateScore {
    pub target_rel_path: String,
    pub similarity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementReferenceSupport {
    pub paper_id: String,
    pub title: String,
    pub similarity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementTargetProfile {
    pub target_rel_path: String,
    pub query_text: String,
    pub source: PlacementTargetProfileSource,
    pub reference_support_count: usize,
    #[serde(default)]
    pub reference_support: Vec<PlacementReferenceSupport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperPlacementEvidence {
    pub file_id: String,
    pub chosen_target_rel_path: String,
    pub decision_source: PlacementDecisionSource,
    #[serde(default)]
    pub top_candidates: Vec<PlacementCandidateScore>,
    pub top_score: Option<f32>,
    pub margin_over_runner_up: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementEvidence {
    pub assistance: PlacementAssistance,
    #[serde(default)]
    pub target_profiles: Vec<PlacementTargetProfile>,
    #[serde(default)]
    pub papers: Vec<PaperPlacementEvidence>,
}

impl PlacementEvidence {
    #[must_use]
    pub fn empty(assistance: PlacementAssistance) -> Self {
        Self {
            assistance,
            target_profiles: Vec::new(),
            papers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlacementEmbeddingOptions {
    pub reference_manifest_path: PathBuf,
    pub reference_top_k: usize,
    pub candidate_top_k: usize,
    pub min_similarity: f32,
    pub min_margin: f32,
    pub min_reference_support: usize,
    pub provider: LlmProvider,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PlacementOptions {
    pub batch_size: usize,
    pub batch_start_delay_ms: u64,
    pub assistance: PlacementAssistance,
    pub placement_mode: PlacementMode,
    pub category_depth: u8,
    pub embedding: Option<PlacementEmbeddingOptions>,
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
pub struct PlacementBatchResult {
    pub batch_index: usize,
    pub file_ids: Vec<String>,
    pub placements: Vec<PlacementDecision>,
    #[serde(default)]
    pub evidence: Vec<PaperPlacementEvidence>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlacementBatchProgress {
    pub completed_batches: Vec<PlacementBatchResult>,
    pub usage: LlmUsageSummary,
}

pub use inspect::inspect_output;
pub use runtime::generate_placements;
pub use runtime::generate_placements_with_progress;
