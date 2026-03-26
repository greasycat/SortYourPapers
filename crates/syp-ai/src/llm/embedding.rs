use async_trait::async_trait;

use crate::{
    error::Result,
    llm::{LlmCallMetrics, LlmProvider, providers},
};

#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    pub provider: LlmProvider,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingInput {
    pub text: String,
}

impl EmbeddingInput {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingRequest {
    pub inputs: Vec<EmbeddingInput>,
}

impl EmbeddingRequest {
    #[must_use]
    pub fn from_texts<I, S>(inputs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            inputs: inputs.into_iter().map(EmbeddingInput::new).collect(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inputs.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingVector {
    pub values: Vec<f32>,
}

impl EmbeddingVector {
    #[must_use]
    pub fn dimensions(&self) -> usize {
        self.values.len()
    }
}

#[derive(Debug, Clone)]
pub struct EmbeddingResponse {
    pub embeddings: Vec<EmbeddingVector>,
    pub metrics: LlmCallMetrics,
}

#[async_trait]
pub trait EmbeddingClient: Send + Sync {
    async fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingResponse>;
}

pub fn build_embedding_client(config: &EmbeddingConfig) -> Result<Box<dyn EmbeddingClient>> {
    Ok(match config.provider {
        LlmProvider::Openai => Box::new(providers::openai::OpenAiClient::new(
            config.model.clone(),
            config.base_url.clone(),
            config.api_key.clone(),
        )),
        LlmProvider::Ollama => Box::new(providers::ollama::OllamaClient::new(
            config.model.clone(),
            config.base_url.clone(),
        )),
        LlmProvider::Gemini => Box::new(providers::gemini::GeminiClient::new(
            config.model.clone(),
            config.base_url.clone(),
            config.api_key.clone(),
        )),
    })
}
