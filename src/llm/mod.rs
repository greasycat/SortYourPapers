use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Duration;
use tokio::time::sleep;

use crate::{
    error::{AppError, Result},
    models::{AppConfig, LlmCallMetrics, LlmProvider},
};

pub mod gemini;
pub mod ollama;
pub mod openai;

const MAX_HTTP_ATTEMPTS: usize = 3;
const HTTP_RETRY_BASE_DELAY_MS: u64 = 500;

#[derive(Debug, Clone)]
pub struct JsonResponseSchema {
    name: &'static str,
    schema: Value,
}

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

impl JsonResponseSchema {
    pub fn new(name: &'static str, schema: Value) -> Self {
        Self { name, schema }
    }

    pub fn name(&self) -> &str {
        self.name
    }

    pub fn schema(&self) -> &Value {
        &self.schema
    }
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
    schema: &JsonResponseSchema,
    max_attempts: usize,
) -> Result<ParsedLlmResponse<T>> {
    let mut prompt = user_prompt.to_string();
    let attempts = max_attempts.max(1);
    let mut last_error = String::new();
    let mut aggregated_metrics: Option<LlmCallMetrics> = None;

    for attempt in 1..=attempts {
        let response = chat_json_with_retry(client, system_prompt, &prompt, schema).await?;
        let mut metrics = response.metrics;
        let normalized = strip_code_fence(&response.content);

        match serde_json::from_str::<T>(&normalized) {
            Ok(v) => {
                let mut total_metrics: LlmCallMetrics = aggregated_metrics.unwrap_or_default();
                total_metrics.merge_from(&metrics);
                return Ok(ParsedLlmResponse {
                    value: v,
                    metrics: total_metrics,
                });
            }
            Err(err) => {
                last_error = err.to_string();
                if attempt < attempts {
                    metrics.json_retry_count += 1;
                }
                let mut total_metrics: LlmCallMetrics = aggregated_metrics.unwrap_or_default();
                total_metrics.merge_from(&metrics);
                aggregated_metrics = Some(total_metrics);
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

pub async fn call_text_with_retry(
    client: &dyn LlmClient,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<LlmResponse> {
    chat_with_retry(client, system_prompt, user_prompt).await
}

async fn chat_json_with_retry(
    client: &dyn LlmClient,
    system_prompt: &str,
    user_prompt: &str,
    schema: &JsonResponseSchema,
) -> Result<LlmResponse> {
    let mut last_error = None;

    for attempt in 1..=MAX_HTTP_ATTEMPTS {
        match client.chat_json(system_prompt, user_prompt, schema).await {
            Ok(mut response) => {
                response.metrics.http_attempt_count = attempt as u64;
                return Ok(response);
            }
            Err(err) if should_retry_llm_http_error(&err) && attempt < MAX_HTTP_ATTEMPTS => {
                last_error = Some(err);
                sleep(http_retry_delay(attempt)).await;
            }
            Err(err) => return Err(err),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        AppError::Execution("chat_json retry loop exited without a result".to_string())
    }))
}

async fn chat_with_retry(
    client: &dyn LlmClient,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<LlmResponse> {
    let mut last_error = None;

    for attempt in 1..=MAX_HTTP_ATTEMPTS {
        match client.chat(system_prompt, user_prompt).await {
            Ok(mut response) => {
                response.metrics.http_attempt_count = attempt as u64;
                return Ok(response);
            }
            Err(err) if should_retry_llm_http_error(&err) && attempt < MAX_HTTP_ATTEMPTS => {
                last_error = Some(err);
                sleep(http_retry_delay(attempt)).await;
            }
            Err(err) => return Err(err),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        AppError::Execution("chat retry loop exited without a result".to_string())
    }))
}

fn should_retry_llm_http_error(err: &AppError) -> bool {
    match err {
        AppError::Http(http_err) => {
            if let Some(status) = http_err.status() {
                return status.as_u16() == 429 || status.is_server_error();
            }

            http_err.is_timeout() || http_err.is_connect() || http_err.is_request()
        }
        _ => false,
    }
}

fn http_retry_delay(attempt: usize) -> Duration {
    let exponent = u32::try_from(attempt.saturating_sub(1)).unwrap_or(u32::MAX);
    let multiplier = 2_u64.saturating_pow(exponent).max(1);
    Duration::from_millis(HTTP_RETRY_BASE_DELAY_MS.saturating_mul(multiplier))
}

pub(crate) fn strip_code_fence(raw: &str) -> String {
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

impl LlmCallMetrics {
    fn merge_from(&mut self, other: &Self) {
        if self.provider.is_empty() {
            self.provider.clone_from(&other.provider);
        }
        if self.model.is_empty() {
            self.model.clone_from(&other.model);
        }
        if self.endpoint_kind.is_empty() {
            self.endpoint_kind.clone_from(&other.endpoint_kind);
        }

        self.request_chars += other.request_chars;
        self.response_chars += other.response_chars;
        self.http_attempt_count += other.http_attempt_count;
        self.json_retry_count += other.json_retry_count;
        self.semantic_retry_count += other.semantic_retry_count;
        self.input_tokens = sum_optional(self.input_tokens, other.input_tokens);
        self.output_tokens = sum_optional(self.output_tokens, other.output_tokens);
        self.total_tokens = sum_optional(self.total_tokens, other.total_tokens);
    }
}

fn sum_optional(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(a), Some(b)) => Some(a + b),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::openai::OpenAiClient;
    use serde_json::{Value, json};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
    };

    enum StubAction {
        Close,
        Respond(String),
    }

    #[tokio::test]
    async fn call_json_with_retry_retries_transport_failures() {
        let (base_url, requests, handle) = spawn_openai_stub(vec![
            StubAction::Close,
            StubAction::Respond(http_response(
                "HTTP/1.1 200 OK",
                r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#,
            )),
        ]);
        let client = OpenAiClient::new("test-model".to_string(), Some(base_url), None);
        let schema = test_response_schema();

        let response: ParsedLlmResponse<Value> =
            call_json_with_retry(&client, "system", "user", &schema, 1)
                .await
                .expect("transport retries should recover");

        assert_eq!(response.value["ok"], true);
        assert_eq!(response.metrics.http_attempt_count, 2);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        handle
            .join()
            .expect("stub server thread should exit cleanly");
    }

    #[tokio::test]
    async fn call_json_with_retry_retries_server_errors() {
        let (base_url, requests, handle) = spawn_openai_stub(vec![
            StubAction::Respond(http_response(
                "HTTP/1.1 500 Internal Server Error",
                r#"{"error":"temporary"}"#,
            )),
            StubAction::Respond(http_response(
                "HTTP/1.1 200 OK",
                r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#,
            )),
        ]);
        let client = OpenAiClient::new("test-model".to_string(), Some(base_url), None);
        let schema = test_response_schema();

        let response: ParsedLlmResponse<Value> =
            call_json_with_retry(&client, "system", "user", &schema, 1)
                .await
                .expect("server status retries should recover");

        assert_eq!(response.value["ok"], true);
        assert_eq!(response.metrics.http_attempt_count, 2);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        handle
            .join()
            .expect("stub server thread should exit cleanly");
    }

    fn spawn_openai_stub(
        actions: Vec<StubAction>,
    ) -> (String, Arc<AtomicUsize>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub server");
        let addr = listener.local_addr().expect("stub server addr");
        let requests = Arc::new(AtomicUsize::new(0));
        let request_counter = Arc::clone(&requests);

        let handle = thread::spawn(move || {
            for action in actions {
                let (mut stream, _) = listener.accept().expect("accept request");
                request_counter.fetch_add(1, Ordering::SeqCst);

                let mut buffer = [0_u8; 8192];
                let _ = stream.read(&mut buffer);

                match action {
                    StubAction::Close => {}
                    StubAction::Respond(response) => {
                        stream
                            .write_all(response.as_bytes())
                            .expect("write response");
                        stream.flush().expect("flush response");
                    }
                }
            }
        });

        (format!("http://{addr}"), requests, handle)
    }

    fn http_response(status_line: &str, body: &str) -> String {
        format!(
            "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn test_response_schema() -> JsonResponseSchema {
        JsonResponseSchema::new(
            "test_response",
            json!({
                "type": "object",
                "properties": {
                    "ok": {
                        "type": "boolean"
                    }
                },
                "required": ["ok"],
                "additionalProperties": false
            }),
        )
    }
}
