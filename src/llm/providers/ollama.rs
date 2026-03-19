use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::error::{AppError, Result};
use crate::llm::LlmCallMetrics;

use crate::llm::{JsonResponseSchema, LlmClient, LlmResponse};

const DEFAULT_BASE_URL: &str = "http://localhost:11434";
const HTTP_CONNECT_TIMEOUT_SECS: u64 = 10;
const HTTP_REQUEST_TIMEOUT_SECS: u64 = 180;

pub struct OllamaClient {
    model: String,
    base_url: String,
    http: reqwest::Client,
}

impl OllamaClient {
    pub fn new(model: String, base_url: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(HTTP_REQUEST_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            model,
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            http,
        }
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<Value>,
    messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    message: Option<MessageResponse>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct MessageResponse {
    content: String,
}

#[async_trait]
impl LlmClient for OllamaClient {
    async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<LlmResponse> {
        self.send_chat(system_prompt, user_prompt, None).await
    }

    async fn chat_json(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        schema: &JsonResponseSchema,
    ) -> Result<LlmResponse> {
        self.send_chat(system_prompt, user_prompt, Some(schema.schema().clone()))
            .await
    }
}

impl OllamaClient {
    async fn send_chat(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        format: Option<Value>,
    ) -> Result<LlmResponse> {
        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));

        let payload = ChatRequest {
            model: self.model.clone(),
            stream: false,
            format,
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: user_prompt.to_string(),
                },
            ],
        };

        let resp = self
            .http
            .post(url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        let body: ChatResponse = resp.json().await?;
        let content = body
            .message
            .map(|m| m.content.trim().to_string())
            .filter(|m| !m.is_empty())
            .ok_or_else(|| AppError::Llm("Ollama response has no message content".to_string()))?;

        let input_tokens = body.prompt_eval_count;
        let output_tokens = body.eval_count;
        let total_tokens = body
            .prompt_eval_count
            .zip(body.eval_count)
            .map(|(input, output)| input + output);

        Ok(LlmResponse {
            metrics: LlmCallMetrics {
                provider: "ollama".to_string(),
                model: self.model.clone(),
                endpoint_kind: "chat".to_string(),
                request_chars: prompt_chars(system_prompt, user_prompt),
                response_chars: content.chars().count() as u64,
                input_tokens,
                output_tokens,
                total_tokens,
                ..LlmCallMetrics::default()
            },
            content,
        })
    }
}

fn prompt_chars(system_prompt: &str, user_prompt: &str) -> u64 {
    (system_prompt.chars().count() + user_prompt.chars().count()) as u64
}
