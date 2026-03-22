use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::error::{AppError, Result};
use crate::llm::LlmCallMetrics;

use crate::llm::{
    EmbeddingClient, EmbeddingRequest, EmbeddingResponse, EmbeddingVector, JsonResponseSchema,
    LlmClient, LlmResponse,
};

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

#[derive(Debug, Serialize)]
struct EmbedRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
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

#[async_trait]
impl EmbeddingClient for OllamaClient {
    async fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        self.send_embeddings(request).await
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

    async fn send_embeddings(&self, request: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        if request.inputs.is_empty() {
            return Err(AppError::Validation(
                "embedding request requires at least one input".to_string(),
            ));
        }

        let url = format!("{}/api/embed", self.base_url.trim_end_matches('/'));
        let payload = EmbedRequest {
            model: self.model.clone(),
            input: request
                .inputs
                .iter()
                .map(|input| input.text.clone())
                .collect(),
        };

        let resp = self
            .http
            .post(url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        let body: EmbedResponse = resp.json().await?;
        let embeddings = body
            .embeddings
            .into_iter()
            .map(|values| EmbeddingVector { values })
            .collect::<Vec<_>>();

        if embeddings.len() != request.inputs.len() {
            return Err(AppError::Llm(format!(
                "Ollama embedding response count {} did not match request count {}",
                embeddings.len(),
                request.inputs.len()
            )));
        }

        Ok(EmbeddingResponse {
            embeddings,
            metrics: LlmCallMetrics {
                provider: "ollama".to_string(),
                model: self.model.clone(),
                endpoint_kind: "embedding".to_string(),
                request_chars: embedding_request_chars(request),
                response_chars: 0,
                input_tokens: body.prompt_eval_count,
                output_tokens: None,
                total_tokens: body.prompt_eval_count,
                ..LlmCallMetrics::default()
            },
        })
    }
}

fn prompt_chars(system_prompt: &str, user_prompt: &str) -> u64 {
    (system_prompt.chars().count() + user_prompt.chars().count()) as u64
}

fn embedding_request_chars(request: &EmbeddingRequest) -> u64 {
    request
        .inputs
        .iter()
        .map(|input| input.text.chars().count() as u64)
        .sum()
}
