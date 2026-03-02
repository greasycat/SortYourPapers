use std::path::PathBuf;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementMode {
    ExistingOnly,
    AllowNew,
}

impl Default for PlacementMode {
    fn default() -> Self {
        Self::ExistingOnly
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    Openai,
    Ollama,
    Gemini,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub input: PathBuf,
    pub output: PathBuf,
    pub recursive: bool,
    pub max_file_size_mb: u64,
    pub page_cutoff: u8,
    pub category_depth: u8,
    pub placement_mode: PlacementMode,
    pub rebuild: bool,
    pub dry_run: bool,
    pub llm_provider: LlmProvider,
    pub llm_model: String,
    pub llm_base_url: Option<String>,
    pub api_key: Option<String>,
    pub keyword_batch_size: usize,
    pub debug: bool,
}

#[derive(Debug, Clone)]
pub struct PdfCandidate {
    pub path: PathBuf,
    pub size_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct PaperText {
    pub file_id: String,
    pub path: PathBuf,
    pub extracted_text: String,
    pub pages_read: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordSet {
    pub file_id: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryTree {
    pub name: String,
    #[serde(default)]
    pub children: Vec<CategoryTree>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementDecision {
    pub file_id: String,
    pub target_rel_path: String,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction {
    Move,
}

#[derive(Debug, Clone)]
pub struct PlanAction {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub action: FileAction,
}

#[derive(Debug, Clone)]
pub struct RunReport {
    pub scanned: usize,
    pub processed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub actions: Vec<PlanAction>,
    pub dry_run: bool,
}

impl RunReport {
    pub fn new(dry_run: bool) -> Self {
        Self {
            scanned: 0,
            processed: 0,
            skipped: 0,
            failed: 0,
            actions: Vec::new(),
            dry_run,
        }
    }
}
