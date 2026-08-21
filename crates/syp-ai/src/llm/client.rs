use async_trait::async_trait;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

use crate::{
    error::{AppError, Result},
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

/// One rendered page handed to a vision-capable model.
///
/// Used to read PDFs that carry no text layer, where the page image is the
/// only content there is.
#[derive(Debug, Clone)]
pub struct PageImage {
    pub mime_type: String,
    pub data: Vec<u8>,
}

impl PageImage {
    #[must_use]
    pub fn png(data: Vec<u8>) -> Self {
        Self {
            mime_type: "image/png".to_string(),
            data,
        }
    }

    /// Base64 payload on its own, the shape Gemini and Ollama expect.
    #[must_use]
    pub fn base64(&self) -> String {
        BASE64.encode(&self.data)
    }

    /// Base64 payload as a `data:` URL, the shape OpenAI expects.
    #[must_use]
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.mime_type, self.base64())
    }
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

    /// Sends page images alongside the prompt.
    ///
    /// Whether the request succeeds depends on the model, not the provider: a
    /// text-only model reports the failure from the API.
    ///
    /// # Errors
    /// Returns an error when the provider cannot send images at all, or when
    /// the request fails.
    async fn chat_with_images(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        images: &[PageImage],
    ) -> Result<LlmResponse> {
        let _ = (system_prompt, user_prompt, images);
        Err(AppError::Llm(
            "this provider cannot send images to a model".to_string(),
        ))
    }
}

/// Whether the experimental `genai`-backed adapter is selected.
///
/// Reading it here keeps the choice in one place while the two backends are
/// being compared; it is not part of the persisted config.
#[must_use]
pub fn genai_backend_selected() -> bool {
    std::env::var(providers::genai_backend::BACKEND_ENV).is_ok_and(|value| {
        value
            .trim()
            .eq_ignore_ascii_case(providers::genai_backend::BACKEND_NAME)
    })
}

pub fn build_client(config: &ChatConfig) -> Result<Box<dyn LlmClient>> {
    if genai_backend_selected() {
        return Ok(Box::new(providers::genai_backend::GenaiClient::new(
            config.provider,
            config.model.clone(),
            config.base_url.clone(),
            config.api_key.clone(),
        )));
    }

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
