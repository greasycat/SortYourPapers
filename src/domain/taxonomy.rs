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
pub struct CategoryTree {
    pub name: String,
    #[serde(default)]
    pub children: Vec<CategoryTree>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementDecision {
    pub file_id: String,
    pub target_rel_path: String,
}
