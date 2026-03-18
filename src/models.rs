use std::path::PathBuf;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementMode {
    #[default]
    ExistingOnly,
    AllowNew,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TaxonomyMode {
    Global,
    #[default]
    BatchMerge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    Openai,
    Ollama,
    Gemini,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub input: PathBuf,
    pub output: PathBuf,
    pub recursive: bool,
    pub max_file_size_mb: u64,
    pub page_cutoff: u8,
    pub pdf_extract_workers: usize,
    pub category_depth: u8,
    pub taxonomy_mode: TaxonomyMode,
    pub taxonomy_batch_size: usize,
    pub placement_batch_size: usize,
    pub placement_mode: PlacementMode,
    pub rebuild: bool,
    pub dry_run: bool,
    pub llm_provider: LlmProvider,
    pub llm_model: String,
    pub llm_base_url: Option<String>,
    pub api_key: Option<String>,
    pub keyword_batch_size: usize,
    pub batch_start_delay_ms: u64,
    #[serde(default)]
    pub verbose: bool,
    #[serde(default)]
    pub debug: bool,
    #[serde(default)]
    pub quiet: bool,
}

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
pub struct CategoryTree {
    pub name: String,
    #[serde(default)]
    pub children: Vec<CategoryTree>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementDecision {
    pub file_id: String,
    pub target_rel_path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmCallMetrics {
    pub provider: String,
    pub model: String,
    pub endpoint_kind: String,
    pub request_chars: u64,
    pub response_chars: u64,
    #[serde(default)]
    pub http_attempt_count: u64,
    #[serde(default)]
    pub json_retry_count: u64,
    #[serde(default)]
    pub semantic_retry_count: u64,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmUsageSummary {
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub endpoint_kinds: Vec<String>,
    #[serde(default)]
    pub call_count: u64,
    #[serde(default)]
    pub http_attempt_count: u64,
    #[serde(default)]
    pub json_retry_count: u64,
    #[serde(default)]
    pub semantic_retry_count: u64,
    #[serde(default)]
    pub request_chars: u64,
    #[serde(default)]
    pub response_chars: u64,
    #[serde(default)]
    pub calls_with_native_tokens: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

impl LlmUsageSummary {
    pub fn record_call(&mut self, metrics: &LlmCallMetrics) {
        push_unique(&mut self.providers, &metrics.provider);
        push_unique(&mut self.models, &metrics.model);
        push_unique(&mut self.endpoint_kinds, &metrics.endpoint_kind);

        self.call_count += 1;
        self.http_attempt_count += metrics.http_attempt_count;
        self.json_retry_count += metrics.json_retry_count;
        self.semantic_retry_count += metrics.semantic_retry_count;
        self.request_chars += metrics.request_chars;
        self.response_chars += metrics.response_chars;

        if metrics.input_tokens.is_some()
            || metrics.output_tokens.is_some()
            || metrics.total_tokens.is_some()
        {
            self.calls_with_native_tokens += 1;
        }
        self.input_tokens += metrics.input_tokens.unwrap_or(0);
        self.output_tokens += metrics.output_tokens.unwrap_or(0);
        self.total_tokens += metrics.total_tokens.unwrap_or(0);
    }

    pub fn merge(&mut self, other: &Self) {
        for provider in &other.providers {
            push_unique(&mut self.providers, provider);
        }
        for model in &other.models {
            push_unique(&mut self.models, model);
        }
        for endpoint_kind in &other.endpoint_kinds {
            push_unique(&mut self.endpoint_kinds, endpoint_kind);
        }

        self.call_count += other.call_count;
        self.http_attempt_count += other.http_attempt_count;
        self.json_retry_count += other.json_retry_count;
        self.semantic_retry_count += other.semantic_retry_count;
        self.request_chars += other.request_chars;
        self.response_chars += other.response_chars;
        self.calls_with_native_tokens += other.calls_with_native_tokens;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.total_tokens += other.total_tokens;
    }

    #[must_use]
    pub fn has_activity(&self) -> bool {
        self.call_count > 0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmRunUsage {
    #[serde(default)]
    pub keywords: LlmUsageSummary,
    #[serde(default)]
    pub taxonomy: LlmUsageSummary,
    #[serde(default)]
    pub placements: LlmUsageSummary,
}

impl LlmRunUsage {
    #[must_use]
    pub fn has_activity(&self) -> bool {
        self.keywords.has_activity()
            || self.taxonomy.has_activity()
            || self.placements.has_activity()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileAction {
    Move,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanAction {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub action: FileAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub scanned: usize,
    pub processed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub actions: Vec<PlanAction>,
    pub dry_run: bool,
    #[serde(default)]
    pub llm_usage: LlmRunUsage,
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
            llm_usage: LlmRunUsage::default(),
        }
    }
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}
