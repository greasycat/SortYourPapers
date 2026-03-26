mod batch;
mod client;
mod embedding;
pub mod providers;
mod retry;
mod schema;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    Openai,
    Ollama,
    Gemini,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmCallMetrics {
    pub provider: String,
    pub model: String,
    pub endpoint_kind: String,
    pub request_chars: u64,
    pub response_chars: u64,
    #[serde(default)]
    pub http_attempt_count: u64,
    #[serde(default)]
    pub json_retry_count: u64,
    #[serde(default)]
    pub semantic_retry_count: u64,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmUsageSummary {
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub endpoint_kinds: Vec<String>,
    #[serde(default)]
    pub call_count: u64,
    #[serde(default)]
    pub http_attempt_count: u64,
    #[serde(default)]
    pub json_retry_count: u64,
    #[serde(default)]
    pub semantic_retry_count: u64,
    #[serde(default)]
    pub request_chars: u64,
    #[serde(default)]
    pub response_chars: u64,
    #[serde(default)]
    pub calls_with_native_tokens: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

impl LlmUsageSummary {
    pub fn record_call(&mut self, metrics: &LlmCallMetrics) {
        push_unique(&mut self.providers, &metrics.provider);
        push_unique(&mut self.models, &metrics.model);
        push_unique(&mut self.endpoint_kinds, &metrics.endpoint_kind);

        self.call_count += 1;
        self.http_attempt_count += metrics.http_attempt_count;
        self.json_retry_count += metrics.json_retry_count;
        self.semantic_retry_count += metrics.semantic_retry_count;
        self.request_chars += metrics.request_chars;
        self.response_chars += metrics.response_chars;

        if metrics.input_tokens.is_some()
            || metrics.output_tokens.is_some()
            || metrics.total_tokens.is_some()
        {
            self.calls_with_native_tokens += 1;
        }
        self.input_tokens += metrics.input_tokens.unwrap_or(0);
        self.output_tokens += metrics.output_tokens.unwrap_or(0);
        self.total_tokens += metrics.total_tokens.unwrap_or(0);
    }

    pub fn merge(&mut self, other: &Self) {
        for provider in &other.providers {
            push_unique(&mut self.providers, provider);
        }
        for model in &other.models {
            push_unique(&mut self.models, model);
        }
        for endpoint_kind in &other.endpoint_kinds {
            push_unique(&mut self.endpoint_kinds, endpoint_kind);
        }

        self.call_count += other.call_count;
        self.http_attempt_count += other.http_attempt_count;
        self.json_retry_count += other.json_retry_count;
        self.semantic_retry_count += other.semantic_retry_count;
        self.request_chars += other.request_chars;
        self.response_chars += other.response_chars;
        self.calls_with_native_tokens += other.calls_with_native_tokens;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.total_tokens += other.total_tokens;
    }

    #[must_use]
    pub fn has_activity(&self) -> bool {
        self.call_count > 0
    }
}

impl LlmCallMetrics {
    pub(crate) fn merge_from(&mut self, other: &Self) {
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmRunUsage {
    #[serde(default)]
    pub keywords: LlmUsageSummary,
    #[serde(default)]
    pub taxonomy: LlmUsageSummary,
    #[serde(default)]
    pub placements: LlmUsageSummary,
}

impl LlmRunUsage {
    #[must_use]
    pub fn has_activity(&self) -> bool {
        self.keywords.has_activity()
            || self.taxonomy.has_activity()
            || self.placements.has_activity()
    }
}

pub use batch::{
    DEFAULT_REQUEST_DISPATCH_DELAY_MS, RequestBatchOptions, run_delayed_concurrent_requests,
};
pub use batch::{batch_dispatch_spacing, wait_for_dispatch_slot};
pub use client::{ChatConfig, LlmClient, LlmResponse, ParsedLlmResponse, build_client};
pub use embedding::{
    EmbeddingClient, EmbeddingConfig, EmbeddingInput, EmbeddingRequest, EmbeddingResponse,
    EmbeddingVector, build_embedding_client,
};
pub use providers::{gemini, ollama, openai};
pub use retry::strip_code_fence;
pub use retry::{call_json_with_retry, call_text_with_retry};
pub use schema::JsonResponseSchema;

fn sum_optional(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(a), Some(b)) => Some(a + b),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
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
