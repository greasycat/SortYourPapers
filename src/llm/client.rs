use async_trait::async_trait;

use crate::{
    domain::{AppConfig, LlmCallMetrics, LlmProvider},
    error::Result,
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

pub fn build_client(config: &AppConfig) -> Box<dyn LlmClient> {
    match config.llm_provider {
        LlmProvider::Openai => Box::new(providers::openai::OpenAiClient::new(
            config.llm_model.clone(),
            config.llm_base_url.clone(),
            config.api_key.clone(),
        )),
        LlmProvider::Ollama => Box::new(providers::ollama::OllamaClient::new(
            config.llm_model.clone(),
            config.llm_base_url.clone(),
        )),
        LlmProvider::Gemini => Box::new(providers::gemini::GeminiClient::new(
            config.llm_model.clone(),
            config.llm_base_url.clone(),
            config.api_key.clone(),
        )),
    }
}
