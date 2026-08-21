//! The `genai` crate behind this workspace's [`LlmClient`] trait.
//!
//! An experiment: `genai` speaks all three providers this project uses, so it
//! can replace the hand-written request bodies without anything above the
//! trait noticing. Selected with `SYP_LLM_BACKEND=genai`.
//!
//! What stays on this side of the trait is everything the hand-written
//! adapters never owned anyway: retries, JSON repair, batch spacing, and the
//! per-call accounting in [`LlmCallMetrics`].

use async_trait::async_trait;
use genai::{
    Client, ModelIden, ServiceTarget,
    adapter::AdapterKind,
    chat::{
        ChatMessage, ChatOptions, ChatRequest, ChatResponseFormat, ContentPart, JsonSpec,
        Usage as GenaiUsage,
    },
    resolver::{AuthData, Endpoint},
};

use crate::error::{AppError, Result};
use crate::llm::{
    EmbeddingClient, EmbeddingRequest, EmbeddingResponse, EmbeddingVector, JsonResponseSchema,
    LlmCallMetrics, LlmClient, LlmProvider, LlmResponse, PageImage,
};

/// Environment variable that switches client construction over to `genai`.
pub const BACKEND_ENV: &str = "SYP_LLM_BACKEND";
pub const BACKEND_NAME: &str = "genai";

pub struct GenaiClient {
    client: Client,
    model: ModelIden,
    provider: LlmProvider,
}

impl GenaiClient {
    /// Builds a client for one provider, model, endpoint and key.
    ///
    /// The provider is pinned rather than inferred from the model name, so a
    /// custom model on an OpenAI-compatible endpoint still routes correctly.
    #[must_use]
    pub fn new(
        provider: LlmProvider,
        model: String,
        base_url: Option<String>,
        api_key: Option<String>,
    ) -> Self {
        let adapter_kind = adapter_kind_for(provider, &model);
        let mut builder = Client::builder();

        if let Some(base_url) = base_url {
            let endpoint = Endpoint::from_owned(normalized_base_url(adapter_kind, &base_url));
            builder = builder.with_service_target_resolver_fn(move |mut target: ServiceTarget| {
                target.endpoint = endpoint.clone();
                Ok(target)
            });
        }

        if let Some(api_key) = api_key {
            builder = builder.with_auth_resolver_fn(move |_: ModelIden| {
                Ok(Some(AuthData::from_single(&api_key)))
            });
        }

        Self {
            client: builder.build(),
            model: ModelIden::new(adapter_kind, model),
            provider,
        }
    }

    async fn send(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        user_content: Vec<ContentPart>,
        options: Option<ChatOptions>,
        endpoint_kind: &str,
    ) -> Result<LlmResponse> {
        let request = ChatRequest::new(vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(user_content),
        ]);

        let response = self
            .client
            .exec_chat(self.model.clone(), request, options.as_ref())
            .await
            .map_err(|err| {
                AppError::Llm(format!("{} request failed: {err}", self.provider_name()))
            })?;

        let usage = response.usage.clone();
        let content = response
            .into_first_text()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .ok_or_else(|| {
                AppError::Llm(format!("{} response has no content", self.provider_name()))
            })?;

        Ok(LlmResponse {
            metrics: LlmCallMetrics {
                provider: self.provider_name().to_string(),
                model: self.model.model_name.to_string(),
                endpoint_kind: endpoint_kind.to_string(),
                request_chars: (system_prompt.chars().count() + user_prompt.chars().count()) as u64,
                response_chars: content.chars().count() as u64,
                ..metrics_from_usage(&usage)
            },
            content,
        })
    }

    fn provider_name(&self) -> &'static str {
        match self.provider {
            LlmProvider::Openai => "openai",
            LlmProvider::Ollama => "ollama",
            LlmProvider::Gemini => "gemini",
        }
    }
}

#[async_trait]
impl LlmClient for GenaiClient {
    async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<LlmResponse> {
        self.send(
            system_prompt,
            user_prompt,
            vec![ContentPart::from_text(user_prompt)],
            None,
            "chat",
        )
        .await
    }

    async fn chat_json(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        schema: &JsonResponseSchema,
    ) -> Result<LlmResponse> {
        let options = ChatOptions::default().with_response_format(ChatResponseFormat::JsonSpec(
            JsonSpec::new(schema.name(), schema.schema().clone()),
        ));
        self.send(
            system_prompt,
            user_prompt,
            vec![ContentPart::from_text(user_prompt)],
            Some(options),
            "chat_json",
        )
        .await
    }

    async fn chat_with_images(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        images: &[PageImage],
    ) -> Result<LlmResponse> {
        let mut content = vec![ContentPart::from_text(user_prompt)];
        content.extend(images.iter().map(|image| {
            ContentPart::from_binary_base64(image.mime_type.clone(), image.base64(), None)
        }));
        self.send(system_prompt, user_prompt, content, None, "chat_vision")
            .await
    }

    fn prefers_plain_text_taxonomy_merge(&self) -> bool {
        // Same provider quirk as the hand-written adapter: Gemini merges
        // taxonomies better when it is not forced into JSON.
        matches!(self.provider, LlmProvider::Gemini)
    }
}

#[async_trait]
impl EmbeddingClient for GenaiClient {
    async fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        if request.inputs.is_empty() {
            return Err(AppError::Validation(
                "embedding request requires at least one input".to_string(),
            ));
        }

        let inputs = request
            .inputs
            .iter()
            .map(|input| input.text.clone())
            .collect::<Vec<_>>();
        let response = self
            .client
            .embed_batch(self.model.clone(), inputs, None)
            .await
            .map_err(|err| {
                AppError::Llm(format!(
                    "{} embedding request failed: {err}",
                    self.provider_name()
                ))
            })?;

        let embeddings = response
            .embeddings
            .into_iter()
            .map(|embedding| EmbeddingVector {
                values: embedding.vector().to_vec(),
            })
            .collect::<Vec<_>>();

        if embeddings.len() != request.inputs.len() {
            return Err(AppError::Llm(format!(
                "{} embedding response count {} did not match request count {}",
                self.provider_name(),
                embeddings.len(),
                request.inputs.len()
            )));
        }

        Ok(EmbeddingResponse {
            embeddings,
            metrics: LlmCallMetrics {
                provider: self.provider_name().to_string(),
                model: self.model.model_name.to_string(),
                endpoint_kind: "embeddings".to_string(),
                request_chars: request
                    .inputs
                    .iter()
                    .map(|input| input.text.chars().count() as u64)
                    .sum(),
                response_chars: 0,
                ..metrics_from_usage(&response.usage)
            },
        })
    }
}

/// Picks the protocol `genai` should speak for one of this project's providers.
///
/// OpenAI splits across two adapters exactly as the hand-written client does,
/// keyed on the same `gpt-5` prefix.
fn adapter_kind_for(provider: LlmProvider, model: &str) -> AdapterKind {
    match provider {
        LlmProvider::Gemini => AdapterKind::Gemini,
        LlmProvider::Ollama => AdapterKind::Ollama,
        LlmProvider::Openai if model.starts_with("gpt-5") => AdapterKind::OpenAIResp,
        LlmProvider::Openai => AdapterKind::OpenAI,
    }
}

/// Trims the endpoint the way `genai` expects it.
///
/// `genai` joins paths onto the endpoint, so it wants a trailing slash where
/// this project's config stores none.
fn normalized_base_url(_adapter_kind: AdapterKind, base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{trimmed}/")
}

fn metrics_from_usage(usage: &GenaiUsage) -> LlmCallMetrics {
    LlmCallMetrics {
        input_tokens: usage.prompt_tokens.map(|tokens| tokens.max(0) as u64),
        output_tokens: usage.completion_tokens.map(|tokens| tokens.max(0) as u64),
        total_tokens: usage.total_tokens.map(|tokens| tokens.max(0) as u64),
        ..LlmCallMetrics::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_models_split_across_the_two_openai_protocols() {
        assert_eq!(
            adapter_kind_for(LlmProvider::Openai, "gpt-5.6-terra"),
            AdapterKind::OpenAIResp
        );
        assert_eq!(
            adapter_kind_for(LlmProvider::Openai, "gpt-4o-mini"),
            AdapterKind::OpenAI
        );
        assert_eq!(
            adapter_kind_for(LlmProvider::Gemini, "gemini-3.7-flash"),
            AdapterKind::Gemini
        );
        assert_eq!(
            adapter_kind_for(LlmProvider::Ollama, "llama3.1"),
            AdapterKind::Ollama
        );
    }

    #[test]
    fn base_urls_are_normalized_for_path_joining() {
        assert_eq!(
            normalized_base_url(AdapterKind::OpenAI, "http://localhost:1234/v1"),
            "http://localhost:1234/v1/"
        );
        assert_eq!(
            normalized_base_url(AdapterKind::OpenAI, "http://localhost:1234/v1/"),
            "http://localhost:1234/v1/"
        );
    }
}

#[cfg(test)]
mod wire_tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    use serde_json::Value;

    use super::*;

    fn stub_server(body: &str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
        let addr = listener.local_addr().expect("addr");
        let (tx, rx) = mpsc::channel();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let read = stream.read(&mut chunk).expect("read");
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
                let text = String::from_utf8_lossy(&buffer).to_string();
                if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                    let len = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .or_else(|| line.strip_prefix("Content-Length:"))
                        })
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if body.len() >= len {
                        break;
                    }
                }
            }
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            tx.send(String::from_utf8_lossy(&buffer).to_string())
                .expect("send");
        });
        (format!("http://{addr}"), rx)
    }

    #[tokio::test]
    async fn gemini_json_requests_carry_the_schema_and_a_system_instruction() {
        let (base_url, rx) = stub_server(
            r#"{"candidates":[{"content":{"parts":[{"text":"{\"ok\":true}"}]}}],"usageMetadata":{"promptTokenCount":11,"candidatesTokenCount":7,"totalTokenCount":18}}"#,
        );
        let client = GenaiClient::new(
            LlmProvider::Gemini,
            "gemini-3.7-flash".to_string(),
            Some(base_url),
            Some("test-key".to_string()),
        );
        let schema = JsonResponseSchema::new(
            "test_response",
            serde_json::json!({"type":"object","properties":{"ok":{"type":"boolean"}}}),
        );

        let response = client
            .chat_json("system prompt", "user prompt", &schema)
            .await
            .expect("structured call should succeed");

        let request = rx.recv().expect("captured request");
        let (head, body) = request.split_once("\r\n\r\n").expect("body");
        let payload: Value = serde_json::from_str(body).expect("json body");

        assert!(head.starts_with("POST /models/gemini-3.7-flash:generateContent"));
        // Unlike the hand-written adapter, genai sends a real systemInstruction
        // instead of folding the system prompt into the user text.
        assert_eq!(
            payload["systemInstruction"]["parts"][0]["text"],
            "system prompt"
        );
        assert_eq!(payload["contents"][0]["parts"][0]["text"], "user prompt");
        assert_eq!(
            payload["generationConfig"]["responseMimeType"],
            "application/json"
        );
        assert_eq!(
            payload["generationConfig"]["responseJsonSchema"]["type"],
            "object"
        );
        assert_eq!(response.content, "{\"ok\":true}");
        assert_eq!(response.metrics.input_tokens, Some(11));
        assert_eq!(response.metrics.output_tokens, Some(7));
        assert_eq!(response.metrics.total_tokens, Some(18));
    }

    #[tokio::test]
    async fn gemini_vision_requests_match_the_hand_written_inline_data_shape() {
        let (base_url, rx) =
            stub_server(r#"{"candidates":[{"content":{"parts":[{"text":"a scanned paper"}]}}]}"#);
        let client = GenaiClient::new(
            LlmProvider::Gemini,
            "gemini-3.7-flash".to_string(),
            Some(base_url),
            Some("test-key".to_string()),
        );
        let images = vec![PageImage::png(b"fake-png".to_vec())];

        let response = client
            .chat_with_images("system prompt", "user prompt", &images)
            .await
            .expect("vision call should succeed");

        let request = rx.recv().expect("captured request");
        let (_, body) = request.split_once("\r\n\r\n").expect("body");
        let payload: Value = serde_json::from_str(body).expect("json body");
        let parts = &payload["contents"][0]["parts"];

        assert_eq!(parts[0]["text"], "user prompt");
        assert_eq!(parts[1]["inline_data"]["mime_type"], "image/png");
        assert_eq!(parts[1]["inline_data"]["data"], images[0].base64());
        assert_eq!(response.content, "a scanned paper");
    }

    /// genai deserializes the Responses body into a typed struct, so it needs a
    /// complete one: `id`, `status`, `model` and a fully shaped `output` are all
    /// required where the hand-written adapter only looks for the first text.
    #[tokio::test]
    async fn openai_responses_need_a_complete_body_but_then_map_cleanly() {
        let (base_url, rx) = stub_server(
            r#"{"id":"resp_1","status":"completed","model":"gpt-5.6-terra","created_at":1,"object":"response","output":[{"id":"msg_1","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"hello","annotations":[]}]}],"usage":{"input_tokens":5,"output_tokens":3,"total_tokens":8}}"#,
        );
        let client = GenaiClient::new(
            LlmProvider::Openai,
            "gpt-5.6-terra".to_string(),
            Some(base_url),
            Some("test-key".to_string()),
        );

        let response = client
            .chat("system prompt", "user prompt")
            .await
            .expect("responses call should succeed");

        let request = rx.recv().expect("captured request");
        let (head, body) = request.split_once("\r\n\r\n").expect("body");
        let payload: Value = serde_json::from_str(body).expect("json body");

        assert!(head.starts_with("POST /responses"));
        assert_eq!(payload["input"][0]["role"], "system");
        assert_eq!(payload["input"][1]["content"], "user prompt");
        assert_eq!(response.content, "hello");
        assert_eq!(response.metrics.total_tokens, Some(8));
    }
}
