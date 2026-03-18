use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::error::{AppError, Result};
use crate::models::LlmCallMetrics;

use crate::llm::{JsonResponseSchema, LlmClient, LlmResponse};

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

    fn use_responses_api(&self) -> bool {
        self.model.starts_with("gpt-5")
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ChatResponseFormat>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
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
struct ChatResponseFormat {
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    json_schema: Option<StructuredJsonSchemaConfig>,
}

#[derive(Debug, Serialize)]
struct StructuredJsonSchemaConfig {
    name: String,
    strict: bool,
    schema: Value,
}

#[derive(Debug, Serialize)]
struct ResponsesRequest {
    model: String,
    input: Vec<ResponseInputMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<ResponseTextConfig>,
}

#[derive(Debug, Serialize)]
struct ResponseInputMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ResponseTextConfig {
    format: ResponseTextFormat,
}

#[derive(Debug, Serialize)]
struct ResponseTextFormat {
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    strict: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ResponsesResponse {
    #[serde(default)]
    output_text: Option<String>,
    #[serde(default)]
    output: Vec<ResponseOutputItem>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct ResponseOutputItem {
    #[serde(default)]
    content: Vec<ResponseContentItem>,
}

#[derive(Debug, Deserialize)]
struct ResponseContentItem {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
}

#[async_trait]
impl LlmClient for OpenAiClient {
    async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<LlmResponse> {
        self.send_chat(system_prompt, user_prompt, None).await
    }

    async fn chat_json(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        schema: &JsonResponseSchema,
    ) -> Result<LlmResponse> {
        self.send_chat(system_prompt, user_prompt, Some(schema))
            .await
    }
}

impl OpenAiClient {
    async fn send_chat(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        response_schema: Option<&JsonResponseSchema>,
    ) -> Result<LlmResponse> {
        if self.use_responses_api() {
            return self
                .send_responses(system_prompt, user_prompt, response_schema)
                .await;
        }

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let headers = self.request_headers()?;
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
            response_format: response_schema.map(chat_response_format),
        };

        let body = self.post_openai_json(&url, headers, &payload).await?;
        let parsed: ChatResponse = serde_json::from_value(body)?;
        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.trim().to_string())
            .filter(|c| !c.is_empty())
            .ok_or_else(|| AppError::Llm("OpenAI response has no content".to_string()))?;

        Ok(LlmResponse {
            metrics: self.call_metrics(
                "chat_completions",
                system_prompt,
                user_prompt,
                &content,
                parsed.usage.as_ref(),
            ),
            content,
        })
    }

    async fn send_responses(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        response_schema: Option<&JsonResponseSchema>,
    ) -> Result<LlmResponse> {
        let url = format!("{}/responses", self.base_url.trim_end_matches('/'));
        let headers = self.request_headers()?;
        let payload = ResponsesRequest {
            model: self.model.clone(),
            input: vec![
                ResponseInputMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                ResponseInputMessage {
                    role: "user".to_string(),
                    content: user_prompt.to_string(),
                },
            ],
            text: response_schema.map(response_text_config),
        };

        let body = self.post_openai_json(&url, headers, &payload).await?;
        let parsed: ResponsesResponse = serde_json::from_value(body)?;
        let usage = parsed.usage.clone();
        let content = parsed
            .output_text
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .or_else(|| {
                parsed
                    .output
                    .into_iter()
                    .flat_map(|item| item.content.into_iter())
                    .filter_map(|item| item.text)
                    .map(|text| text.trim().to_string())
                    .find(|text| !text.is_empty())
            })
            .ok_or_else(|| AppError::Llm("OpenAI response has no content".to_string()))?;

        Ok(LlmResponse {
            metrics: self.call_metrics(
                "responses",
                system_prompt,
                user_prompt,
                &content,
                usage.as_ref(),
            ),
            content,
        })
    }

    fn request_headers(&self) -> Result<HeaderMap> {
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

        Ok(headers)
    }

    fn call_metrics(
        &self,
        endpoint_kind: &str,
        system_prompt: &str,
        user_prompt: &str,
        content: &str,
        usage: Option<&OpenAiUsage>,
    ) -> LlmCallMetrics {
        let usage = usage.cloned().unwrap_or_default();
        let input_tokens = usage.input_tokens.or(usage.prompt_tokens);
        let output_tokens = usage.output_tokens.or(usage.completion_tokens);
        let total_tokens = usage.total_tokens.or_else(|| {
            input_tokens
                .zip(output_tokens)
                .map(|(input, output)| input + output)
        });

        LlmCallMetrics {
            provider: "openai".to_string(),
            model: self.model.clone(),
            endpoint_kind: endpoint_kind.to_string(),
            request_chars: prompt_chars(system_prompt, user_prompt),
            response_chars: content.chars().count() as u64,
            input_tokens,
            output_tokens,
            total_tokens,
            ..LlmCallMetrics::default()
        }
    }
}

fn chat_response_format(schema: &JsonResponseSchema) -> ChatResponseFormat {
    ChatResponseFormat {
        kind: "json_schema".to_string(),
        json_schema: Some(StructuredJsonSchemaConfig {
            name: schema.name().to_string(),
            strict: true,
            schema: schema.schema().clone(),
        }),
    }
}

fn response_text_config(schema: &JsonResponseSchema) -> ResponseTextConfig {
    ResponseTextConfig {
        format: ResponseTextFormat {
            kind: "json_schema".to_string(),
            name: Some(schema.name().to_string()),
            strict: Some(true),
            schema: Some(schema.schema().clone()),
        },
    }
}
impl OpenAiClient {
    async fn post_openai_json<T: Serialize>(
        &self,
        url: &str,
        headers: HeaderMap,
        payload: &T,
    ) -> Result<Value> {
        let resp = self
            .http
            .post(url)
            .headers(headers)
            .json(payload)
            .send()
            .await?;

        if let Err(err) = resp.error_for_status_ref() {
            if err
                .status()
                .map(|status| status.as_u16() == 429 || status.is_server_error())
                .unwrap_or(false)
            {
                return Err(AppError::Http(err));
            }

            let status = err
                .status()
                .map(|status| status.as_u16().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Llm(format!(
                "OpenAI API request failed with status {status}: {}",
                extract_openai_error_message(&body)
            )));
        }

        Ok(resp.json().await?)
    }
}

fn extract_openai_error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .or_else(|| {
            let trimmed = body.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .unwrap_or_else(|| "empty error response".to_string())
}

fn prompt_chars(system_prompt: &str, user_prompt: &str) -> u64 {
    (system_prompt.chars().count() + user_prompt.chars().count()) as u64
}

#[cfg(test)]
mod tests {
    use super::OpenAiClient;
    use crate::llm::{JsonResponseSchema, LlmClient};
    use serde_json::{Value, json};
    use std::{
        env,
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };
    use tokio::time::timeout;

    #[tokio::test]
    async fn gpt5_chat_json_uses_responses_structured_outputs() {
        let (base_url, request_rx, handle) = spawn_single_request_server(
            "HTTP/1.1 200 OK",
            r#"{"output_text":"{\"ok\":true}","usage":{"input_tokens":11,"output_tokens":7,"total_tokens":18}}"#,
        );
        let client = OpenAiClient::new("gpt-5-mini".to_string(), Some(base_url), None);
        let schema = test_response_schema();

        let body = client
            .chat_json("system prompt", "user prompt", &schema)
            .await
            .expect("gpt-5 responses call should succeed");

        let request = request_rx.recv().expect("captured request");
        let request_str = String::from_utf8(request).expect("utf8 request");
        let payload = request_body_json(&request_str);

        assert!(request_str.starts_with("POST /responses HTTP/1.1"));
        assert_eq!(payload["model"], "gpt-5-mini");
        assert!(payload.get("temperature").is_none());
        assert_eq!(payload["input"][0]["role"], "system");
        assert_eq!(payload["input"][1]["role"], "user");
        assert_eq!(payload["text"]["format"]["type"], "json_schema");
        assert_eq!(payload["text"]["format"]["name"], "test_response");
        assert_eq!(payload["text"]["format"]["strict"], true);
        assert_eq!(payload["text"]["format"]["schema"]["type"], "object");
        assert_eq!(body.content, "{\"ok\":true}");
        assert_eq!(body.metrics.input_tokens, Some(11));
        assert_eq!(body.metrics.output_tokens, Some(7));
        assert_eq!(body.metrics.total_tokens, Some(18));

        handle.join().expect("server thread");
    }

    #[tokio::test]
    async fn gpt5_chat_json_sends_project_taxonomy_prompt_and_flat_schema() {
        let (base_url, request_rx, handle) = spawn_single_request_server(
            "HTTP/1.1 200 OK",
            r#"{"output_text":"{\"categories\":[[\"Speech Recognition\"],[\"Speech Recognition\",\"Acoustic Modelling\"]]}","usage":{"input_tokens":19,"output_tokens":9,"total_tokens":28}}"#,
        );
        let client = OpenAiClient::new("gpt-5-mini".to_string(), Some(base_url), None);
        let schema = flat_taxonomy_schema();
        let system =
            "You design hierarchical folder taxonomies for academic PDFs. Return strict JSON only.";
        let user = "Return JSON with schema:\n{\"categories\":[[\"Top Level\"],[\"Top Level\",\"Subcategory\"]]}\nRules:\n- each entry in `categories` must be a full category path from root to a category node\n- include parent paths before child paths\n- do not compress the taxonomy just to satisfy an artificial depth target; the final merge stage will handle any depth reduction\n- names must be filesystem-friendly (letters, numbers, spaces, dashes)\n- avoid duplicate category paths\n- output at least one top-level category path\n\nkeywords:\n[\"acoustic modelling\",\"connectionist temporal classification\",\"deep recurrent neural networks\",\"hidden markov models\",\"long short term memory\",\"lstm\",\"sequence labelling\",\"speech recognition\"]";

        let body = client
            .chat_json(system, user, &schema)
            .await
            .expect("gpt-5 taxonomy request should succeed");

        let request = request_rx.recv().expect("captured request");
        let request_str = String::from_utf8(request).expect("utf8 request");
        let payload = request_body_json(&request_str);

        assert!(request_str.starts_with("POST /responses HTTP/1.1"));
        assert_eq!(payload["input"][0]["content"], system);
        assert_eq!(payload["input"][1]["content"], user);
        assert_eq!(payload["text"]["format"]["type"], "json_schema");
        assert_eq!(payload["text"]["format"]["name"], "category_response");
        assert_eq!(payload["text"]["format"]["strict"], true);
        assert_eq!(
            payload["text"]["format"]["schema"]["properties"]["categories"]["items"]["type"],
            "array"
        );
        assert_eq!(
            payload["text"]["format"]["schema"]["properties"]["categories"]["items"]["items"]["type"],
            "string"
        );
        assert_eq!(
            body.content,
            "{\"categories\":[[\"Speech Recognition\"],[\"Speech Recognition\",\"Acoustic Modelling\"]]}"
        );

        handle.join().expect("server thread");
    }

    #[tokio::test]
    async fn legacy_models_keep_chat_completions_shape() {
        let (base_url, request_rx, handle) = spawn_single_request_server(
            "HTTP/1.1 200 OK",
            r#"{"choices":[{"message":{"content":"ok"}}]}"#,
        );
        let client = OpenAiClient::new("gpt-4o-mini".to_string(), Some(base_url), None);

        let body = client
            .chat("system prompt", "user prompt")
            .await
            .expect("chat completions call should succeed");

        let request = request_rx.recv().expect("captured request");
        let request_str = String::from_utf8(request).expect("utf8 request");
        let payload = request_body_json(&request_str);

        assert!(request_str.starts_with("POST /chat/completions HTTP/1.1"));
        assert_eq!(payload["temperature"], 0.0);
        assert_eq!(payload["messages"][0]["role"], "system");
        assert_eq!(payload["messages"][1]["role"], "user");
        assert_eq!(body.content, "ok");

        handle.join().expect("server thread");
    }

    #[tokio::test]
    async fn chat_completions_chat_json_uses_structured_outputs() {
        let (base_url, request_rx, handle) = spawn_single_request_server(
            "HTTP/1.1 200 OK",
            r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}],"usage":{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}}"#,
        );
        let client = OpenAiClient::new("gpt-4o-mini".to_string(), Some(base_url), None);
        let schema = test_response_schema();

        let body = client
            .chat_json("system prompt", "user prompt", &schema)
            .await
            .expect("chat completions structured output should succeed");

        let request = request_rx.recv().expect("captured request");
        let request_str = String::from_utf8(request).expect("utf8 request");
        let payload = request_body_json(&request_str);

        assert!(request_str.starts_with("POST /chat/completions HTTP/1.1"));
        assert_eq!(payload["response_format"]["type"], "json_schema");
        assert_eq!(
            payload["response_format"]["json_schema"]["name"],
            "test_response"
        );
        assert_eq!(payload["response_format"]["json_schema"]["strict"], true);
        assert_eq!(
            payload["response_format"]["json_schema"]["schema"]["type"],
            "object"
        );
        assert_eq!(body.content, "{\"ok\":true}");
        assert_eq!(body.metrics.input_tokens, Some(5));
        assert_eq!(body.metrics.output_tokens, Some(3));
        assert_eq!(body.metrics.total_tokens, Some(8));

        handle.join().expect("server thread");
    }

    #[tokio::test]
    async fn chat_completions_taxonomy_request_uses_flat_schema() {
        let (base_url, request_rx, handle) = spawn_single_request_server(
            "HTTP/1.1 200 OK",
            r#"{"choices":[{"message":{"content":"{\"categories\":[[\"Speech Recognition\"],[\"Speech Recognition\",\"Acoustic Modelling\"]]}"}}],"usage":{"prompt_tokens":8,"completion_tokens":5,"total_tokens":13}}"#,
        );
        let client = OpenAiClient::new("gpt-4o-mini".to_string(), Some(base_url), None);
        let schema = flat_taxonomy_schema();
        let system =
            "You design hierarchical folder taxonomies for academic PDFs. Return strict JSON only.";
        let user = "Return JSON with schema:\n{\"categories\":[[\"Top Level\"],[\"Top Level\",\"Subcategory\"]]}\nRules:\n- each entry in `categories` must be a full category path from root to a category node\n- include parent paths before child paths\n- do not compress the taxonomy just to satisfy an artificial depth target; the final merge stage will handle any depth reduction\n- names must be filesystem-friendly (letters, numbers, spaces, dashes)\n- avoid duplicate category paths\n- output at least one top-level category path\n\nkeywords:\n[\"acoustic modelling\",\"connectionist temporal classification\",\"deep recurrent neural networks\",\"hidden markov models\",\"long short term memory\",\"lstm\",\"sequence labelling\",\"speech recognition\"]";

        let body = client
            .chat_json(system, user, &schema)
            .await
            .expect("chat completions taxonomy request should succeed");

        let request = request_rx.recv().expect("captured request");
        let request_str = String::from_utf8(request).expect("utf8 request");
        let payload = request_body_json(&request_str);

        assert!(request_str.starts_with("POST /chat/completions HTTP/1.1"));
        assert_eq!(payload["messages"][0]["content"], system);
        assert_eq!(payload["messages"][1]["content"], user);
        assert_eq!(payload["response_format"]["type"], "json_schema");
        assert_eq!(
            payload["response_format"]["json_schema"]["name"],
            "category_response"
        );
        assert_eq!(payload["response_format"]["json_schema"]["strict"], true);
        assert_eq!(
            payload["response_format"]["json_schema"]["schema"]["properties"]["categories"]["items"]
                ["type"],
            "array"
        );
        assert_eq!(
            payload["response_format"]["json_schema"]["schema"]["properties"]["categories"]["items"]
                ["items"]["type"],
            "string"
        );
        assert_eq!(
            body.content,
            "{\"categories\":[[\"Speech Recognition\"],[\"Speech Recognition\",\"Acoustic Modelling\"]]}"
        );

        handle.join().expect("server thread");
    }

    #[tokio::test]
    async fn openai_4xx_surfaces_error_message() {
        let (base_url, _request_rx, handle) = spawn_single_request_server(
            "HTTP/1.1 400 Bad Request",
            r#"{"error":{"message":"temperature is not supported"}}"#,
        );
        let client = OpenAiClient::new("gpt-5-mini".to_string(), Some(base_url), None);

        let err = client
            .chat("system prompt", "user prompt")
            .await
            .expect_err("request should fail");

        assert!(err.to_string().contains("temperature is not supported"));

        handle.join().expect("server thread");
    }

    #[tokio::test]
    #[ignore = "requires SYP_API_KEY and network access; run explicitly"]
    async fn live_openai_taxonomy_request_returns_non_empty_categories() {
        let api_key =
            env::var("SYP_API_KEY").expect("set SYP_API_KEY to run the live OpenAI adapter test");
        let model = env::var("SYP_LLM_MODEL").unwrap_or_else(|_| "gpt-5-mini".to_string());
        let base_url = env::var("SYP_LLM_BASE_URL").ok();

        let client = OpenAiClient::new(model, base_url, Some(api_key));
        let schema = flat_taxonomy_schema();
        let system =
            "You design hierarchical folder taxonomies for academic PDFs. Return strict JSON only.";
        let user = "Return JSON with schema:\n{\"categories\":[[\"Top Level\"],[\"Top Level\",\"Subcategory\"]]}\nRules:\n- each entry in `categories` must be a full category path from root to a category node\n- include parent paths before child paths\n- do not compress the taxonomy just to satisfy an artificial depth target; the final merge stage will handle any depth reduction\n- names must be filesystem-friendly (letters, numbers, spaces, dashes)\n- avoid duplicate category paths\n- output at least one top-level category path\n\nkeywords:\n[\"acoustic modelling\",\"connectionist temporal classification\",\"deep recurrent neural networks\",\"hidden markov models\",\"long short term memory\",\"lstm\",\"sequence labelling\",\"speech recognition\"]";

        let response = timeout(
            Duration::from_secs(90),
            client.chat_json(system, user, &schema),
        )
        .await
        .expect("live OpenAI request exceeded 90s timeout")
        .expect("live OpenAI request should succeed");
        println!(
            "live taxonomy openai adapter response: {}",
            response.content
        );

        let parsed: Value =
            serde_json::from_str(&response.content).expect("response content should be valid json");
        let categories = parsed["categories"]
            .as_array()
            .expect("response should contain a categories array");
        assert!(
            !categories.is_empty(),
            "live taxonomy response should contain at least one top-level category"
        );
        assert!(
            categories.iter().all(|category| {
                category.as_array().is_some_and(|segments| {
                    !segments.is_empty() && segments.iter().all(Value::is_string)
                })
            }),
            "every category should be a non-empty string path"
        );
    }

    fn spawn_single_request_server(
        status_line: &str,
        body: &str,
    ) -> (String, mpsc::Receiver<Vec<u8>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub server");
        let addr = listener.local_addr().expect("local addr");
        let (request_tx, request_rx) = mpsc::channel();
        let response = format!(
            "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let buffer = read_http_request(&mut stream);
            request_tx.send(buffer).expect("send request bytes");
            stream
                .write_all(response.as_bytes())
                .expect("write response");
            stream.flush().expect("flush response");
        });

        (format!("http://{addr}"), request_rx, handle)
    }

    fn request_body_json(request: &str) -> Value {
        let (_, body) = request
            .split_once("\r\n\r\n")
            .expect("http request should contain body");
        serde_json::from_str(body).expect("request body should be json")
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        let mut expected_total_len = None;

        loop {
            let bytes_read = stream.read(&mut chunk).expect("read request chunk");
            if bytes_read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..bytes_read]);

            if expected_total_len.is_none() {
                expected_total_len = parse_expected_request_len(&buffer);
            }

            if let Some(expected_total_len) = expected_total_len {
                if buffer.len() >= expected_total_len {
                    break;
                }
            }
        }

        buffer
    }

    fn parse_expected_request_len(buffer: &[u8]) -> Option<usize> {
        let request = std::str::from_utf8(buffer).ok()?;
        let (headers, body) = request.split_once("\r\n\r\n")?;
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length:")
                    .map(str::trim)
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(0);
        Some(headers.len() + 4 + content_length.max(body.len()))
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

    fn flat_taxonomy_schema() -> JsonResponseSchema {
        JsonResponseSchema::new(
            "category_response",
            json!({
                "type": "object",
                "properties": {
                    "categories": {
                        "type": "array",
                        "items": {
                            "type": "array",
                            "items": {
                                "type": "string"
                            }
                        }
                    }
                },
                "required": ["categories"],
                "additionalProperties": false
            }),
        )
    }
}
