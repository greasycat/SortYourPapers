use async_trait::async_trait;
use serde::de::DeserializeOwned;

use crate::{
    error::{AppError, Result},
    models::{AppConfig, LlmProvider},
};

pub mod gemini;
pub mod ollama;
pub mod openai;

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<String>;

    async fn chat_json(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        self.chat(system_prompt, user_prompt).await
    }
}

pub fn build_client(config: &AppConfig) -> Box<dyn LlmClient> {
    match config.llm_provider {
        LlmProvider::Openai => Box::new(openai::OpenAiClient::new(
            config.llm_model.clone(),
            config.llm_base_url.clone(),
            config.api_key.clone(),
        )),
        LlmProvider::Ollama => Box::new(ollama::OllamaClient::new(
            config.llm_model.clone(),
            config.llm_base_url.clone(),
        )),
        LlmProvider::Gemini => Box::new(gemini::GeminiClient::new(
            config.llm_model.clone(),
            config.llm_base_url.clone(),
            config.api_key.clone(),
        )),
    }
}

pub async fn call_json_with_retry<T: DeserializeOwned>(
    client: &dyn LlmClient,
    system_prompt: &str,
    user_prompt: &str,
    max_attempts: usize,
) -> Result<T> {
    let mut prompt = user_prompt.to_string();
    let attempts = max_attempts.max(1);
    let mut last_error = String::new();

    for attempt in 1..=attempts {
        let content = client.chat_json(system_prompt, &prompt).await?;
        let normalized = strip_code_fence(&content);

        match serde_json::from_str::<T>(&normalized) {
            Ok(v) => return Ok(v),
            Err(err) => {
                last_error = err.to_string();
                if attempt < attempts {
                    prompt = format!(
                        "{user_prompt}\n\nYour previous response was invalid JSON ({last_error}). Return ONLY valid JSON matching the requested schema, with no markdown fences."
                    );
                }
            }
        }
    }

    Err(AppError::Llm(format!(
        "failed to parse model JSON output after {attempts} attempts: {last_error}"
    )))
}

fn strip_code_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }

    let lines: Vec<&str> = trimmed.lines().collect();
    if lines.len() < 3 {
        return trimmed.to_string();
    }

    let start = 1;
    let end = lines
        .iter()
        .rposition(|line| line.trim_start().starts_with("```"))
        .unwrap_or(lines.len());

    lines[start..end].join("\n")
}
