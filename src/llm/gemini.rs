use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::error::{AppError, Result};

use super::LlmClient;

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const HTTP_CONNECT_TIMEOUT_SECS: u64 = 10;
const HTTP_REQUEST_TIMEOUT_SECS: u64 = 180;

pub struct GeminiClient {
    model: String,
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl GeminiClient {
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

    fn normalized_model(&self) -> &str {
        self.model.strip_prefix("models/").unwrap_or(&self.model)
    }
}

#[derive(Debug, Serialize)]
struct GenerateContentRequest {
    #[serde(rename = "systemInstruction")]
    system_instruction: Content,
    contents: Vec<Content>,
    #[serde(rename = "generationConfig")]
    generation_config: GenerationConfig,
}

#[derive(Debug, Serialize, Deserialize)]
struct Content {
    role: Option<String>,
    parts: Vec<Part>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Part {
    text: String,
}

#[derive(Debug, Serialize)]
struct GenerationConfig {
    temperature: f32,
    #[serde(rename = "responseMimeType", skip_serializing_if = "Option::is_none")]
    response_mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GenerateContentResponse {
    candidates: Option<Vec<Candidate>>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: Option<Content>,
}

#[async_trait]
impl LlmClient for GeminiClient {
    async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        self.send_chat(system_prompt, user_prompt, None).await
    }

    async fn chat_json(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        self.send_chat(
            system_prompt,
            user_prompt,
            Some("application/json".to_string()),
        )
        .await
    }
}

impl GeminiClient {
    async fn send_chat(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        response_mime_type: Option<String>,
    ) -> Result<String> {
        let api_key = self
            .api_key
            .as_deref()
            .ok_or(AppError::MissingConfig("api_key (required for gemini)"))?;

        let url = format!(
            "{}/models/{}:generateContent",
            self.base_url.trim_end_matches('/'),
            self.normalized_model()
        );

        let payload = GenerateContentRequest {
            system_instruction: Content {
                role: None,
                parts: vec![Part {
                    text: system_prompt.to_string(),
                }],
            },
            contents: vec![Content {
                role: Some("user".to_string()),
                parts: vec![Part {
                    text: user_prompt.to_string(),
                }],
            }],
            generation_config: GenerationConfig {
                temperature: 0.0,
                response_mime_type,
            },
        };

        let resp = self
            .http
            .post(url)
            .query(&[("key", api_key)])
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        let body: GenerateContentResponse = resp.json().await?;

        let content = body
            .candidates
            .and_then(|candidates| candidates.into_iter().next())
            .and_then(|candidate| candidate.content)
            .and_then(|content| {
                content
                    .parts
                    .into_iter()
                    .map(|part| part.text)
                    .find(|text| !text.trim().is_empty())
            })
            .map(|text| text.trim().to_string())
            .ok_or_else(|| AppError::Llm("Gemini response has no content".to_string()))?;

        Ok(content)
    }
}
