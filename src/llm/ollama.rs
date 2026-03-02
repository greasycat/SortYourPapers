use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::error::{AppError, Result};

use super::LlmClient;

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
}

#[derive(Debug, Deserialize)]
struct MessageResponse {
    content: String,
}

#[async_trait]
impl LlmClient for OllamaClient {
    async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        self.send_chat(system_prompt, user_prompt, None).await
    }

    async fn chat_json(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        self.send_chat(
            system_prompt,
            user_prompt,
            Some(serde_json::Value::String("json".to_string())),
        )
        .await
    }
}

impl OllamaClient {
    async fn send_chat(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        format: Option<Value>,
    ) -> Result<String> {
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

        Ok(content)
    }
}
