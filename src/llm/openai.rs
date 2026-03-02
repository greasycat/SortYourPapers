use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::error::{AppError, Result};

use super::LlmClient;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const HTTP_CONNECT_TIMEOUT_SECS: u64 = 10;
const HTTP_REQUEST_TIMEOUT_SECS: u64 = 180;

pub struct OpenAiClient {
    model: String,
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(model: String, base_url: Option<String>, api_key: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(HTTP_REQUEST_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            model,
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            api_key,
            http,
        }
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChoiceMessage {
    content: String,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: String,
}

#[async_trait]
impl LlmClient for OpenAiClient {
    async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        self.send_chat(system_prompt, user_prompt, None).await
    }

    async fn chat_json(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        self.send_chat(
            system_prompt,
            user_prompt,
            Some(ResponseFormat {
                kind: "json_object".to_string(),
            }),
        )
        .await
    }
}

impl OpenAiClient {
    async fn send_chat(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        response_format: Option<ResponseFormat>,
    ) -> Result<String> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(api_key) = &self.api_key {
            let value = format!("Bearer {api_key}");
            let mut header = HeaderValue::from_str(&value).map_err(|e| {
                AppError::Config(format!("invalid API key for authorization header: {e}"))
            })?;
            header.set_sensitive(true);
            headers.insert(AUTHORIZATION, header);
        }

        let payload = ChatRequest {
            model: self.model.clone(),
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
            temperature: 0.0,
            response_format,
        };

        let resp = self
            .http
            .post(url)
            .headers(headers)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        let body: ChatResponse = resp.json().await?;
        let content = body
            .choices
            .first()
            .map(|c| c.message.content.trim().to_string())
            .filter(|c| !c.is_empty())
            .ok_or_else(|| AppError::Llm("OpenAI response has no content".to_string()))?;

        Ok(content)
    }
}
