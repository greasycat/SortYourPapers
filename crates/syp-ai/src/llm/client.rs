use async_trait::async_trait;

use crate::{
    error::Result,
    llm::{LlmCallMetrics, LlmProvider},
};

use super::{providers, schema::JsonResponseSchema};

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub metrics: LlmCallMetrics,
}

#[derive(Debug)]
pub struct ParsedLlmResponse<T> {
    pub value: T,
    pub metrics: LlmCallMetrics,
}

#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub provider: LlmProvider,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<LlmResponse>;

    fn prefers_plain_text_taxonomy_merge(&self) -> bool {
        false
    }

    async fn chat_json(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        _schema: &JsonResponseSchema,
    ) -> Result<LlmResponse> {
        self.chat(system_prompt, user_prompt).await
    }
}

pub fn build_client(config: &ChatConfig) -> Result<Box<dyn LlmClient>> {
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
