use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::time::Duration;

use crate::error::{AppError, Result};
use crate::models::LlmCallMetrics;

use crate::llm::{JsonResponseSchema, LlmClient, LlmResponse};

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
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<Content>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(rename = "responseMimeType", skip_serializing_if = "Option::is_none")]
    response_mime_type: Option<String>,
    #[serde(rename = "responseSchema", skip_serializing_if = "Option::is_none")]
    response_schema: Option<serde_json::Value>,
    #[serde(rename = "thinkingConfig", skip_serializing_if = "Option::is_none")]
    thinking_config: Option<ThinkingConfig>,
}

#[derive(Debug, Serialize)]
struct ThinkingConfig {
    #[serde(rename = "thinkingLevel")]
    thinking_level: String,
}

#[derive(Debug, Deserialize)]
struct GenerateContentResponse {
    candidates: Option<Vec<Candidate>>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: Option<Content>,
}

#[derive(Debug, Default, Deserialize)]
struct GeminiUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: Option<u64>,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: Option<u64>,
    #[serde(rename = "totalTokenCount")]
    total_token_count: Option<u64>,
}

#[async_trait]
impl LlmClient for GeminiClient {
    async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<LlmResponse> {
        self.send_chat(system_prompt, user_prompt, None, None).await
    }

    fn prefers_plain_text_taxonomy_merge(&self) -> bool {
        true
    }

    async fn chat_json(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        schema: &JsonResponseSchema,
    ) -> Result<LlmResponse> {
        self.send_chat(
            system_prompt,
            user_prompt,
            Some("application/json".to_string()),
            Some(gemini_response_schema(schema.schema())),
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
        response_schema: Option<serde_json::Value>,
    ) -> Result<LlmResponse> {
        let api_key = self
            .api_key
            .as_deref()
            .ok_or(AppError::MissingConfig("api_key (required for gemini)"))?;

        let url = format!(
            "{}/models/{}:generateContent",
            self.base_url.trim_end_matches('/'),
            self.normalized_model()
        );

        let combined_prompt = combine_prompts(system_prompt, user_prompt);
        let payload = GenerateContentRequest {
            system_instruction: None,
            contents: vec![Content {
                role: Some("user".to_string()),
                parts: vec![Part {
                    text: combined_prompt,
                }],
            }],
            generation_config: GenerationConfig {
                temperature: None,
                response_mime_type,
                response_schema,
                thinking_config: Some(ThinkingConfig {
                    thinking_level: "MEDIUM".to_string(),
                }),
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
        let usage_metadata = body.usage_metadata;

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

        Ok(LlmResponse {
            metrics: LlmCallMetrics {
                provider: "gemini".to_string(),
                model: self.model.clone(),
                endpoint_kind: "generate_content".to_string(),
                request_chars: prompt_chars(system_prompt, user_prompt),
                response_chars: content.chars().count() as u64,
                input_tokens: usage_metadata
                    .as_ref()
                    .and_then(|usage| usage.prompt_token_count),
                output_tokens: usage_metadata
                    .as_ref()
                    .and_then(|usage| usage.candidates_token_count),
                total_tokens: usage_metadata
                    .as_ref()
                    .and_then(|usage| usage.total_token_count),
                ..LlmCallMetrics::default()
            },
            content,
        })
    }
}

fn prompt_chars(system_prompt: &str, user_prompt: &str) -> u64 {
    (system_prompt.chars().count() + user_prompt.chars().count()) as u64
}

fn combine_prompts(system_prompt: &str, user_prompt: &str) -> String {
    format!("{system_prompt}\n\n{user_prompt}")
}

fn gemini_response_schema(schema: &Value) -> Value {
    let Some(schema_object) = schema.as_object() else {
        return schema.clone();
    };

    let mut converted = Map::new();

    if let Some(schema_type) = schema_object.get("type").cloned() {
        converted.insert("type".to_string(), schema_type);
    }
    if let Some(properties) = schema_object.get("properties").and_then(Value::as_object) {
        let mut converted_properties = Map::new();
        let mut property_ordering = Vec::with_capacity(properties.len());
        for (key, value) in properties {
            property_ordering.push(Value::String(key.clone()));
            converted_properties.insert(key.clone(), gemini_response_schema(value));
        }
        converted.insert(
            "properties".to_string(),
            Value::Object(converted_properties),
        );
        converted.insert(
            "propertyOrdering".to_string(),
            Value::Array(property_ordering),
        );
    }
    if let Some(items) = schema_object.get("items") {
        converted.insert("items".to_string(), gemini_response_schema(items));
    }
    if let Some(enum_values) = schema_object.get("enum").cloned() {
        converted.insert("enum".to_string(), enum_values);
    }

    Value::Object(converted)
}

#[cfg(test)]
mod tests {
    use super::*;
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
    async fn chat_json_sends_response_schema_to_gemini() {
        let (base_url, request_rx, handle) = spawn_single_request_server(
            "HTTP/1.1 200 OK",
            r#"{
                "candidates":[{"content":{"parts":[{"text":"{\"ok\":true}"}]}}],
                "usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":3,"totalTokenCount":10}
            }"#,
        );
        let client = GeminiClient::new(
            "gemini-2.5-flash".to_string(),
            Some(base_url),
            Some("test-key".to_string()),
        );
        let schema = test_response_schema();

        let response = client
            .chat_json("system prompt", "user prompt", &schema)
            .await
            .expect("request should succeed");

        assert_eq!(response.content, r#"{"ok":true}"#);
        assert_eq!(response.metrics.input_tokens, Some(7));
        assert_eq!(response.metrics.output_tokens, Some(3));
        assert_eq!(response.metrics.total_tokens, Some(10));

        let request = request_rx.recv().expect("captured request");
        let request = String::from_utf8(request).expect("request should be utf8");
        let body = request_body_json(&request);

        assert_eq!(
            body["generationConfig"]["responseMimeType"],
            Value::String("application/json".to_string())
        );
        assert_eq!(
            body["generationConfig"]["responseSchema"],
            gemini_response_schema(schema.schema())
        );
        assert!(body["generationConfig"]["responseJsonSchema"].is_null());
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "MEDIUM"
        );
        assert!(body["systemInstruction"].is_null());
        assert_eq!(
            body["contents"][0]["parts"][0]["text"],
            "system prompt\n\nuser prompt"
        );
        assert!(body["generationConfig"]["temperature"].is_null());

        handle.join().expect("server thread");
    }

    #[tokio::test]
    async fn chat_json_preserves_flat_taxonomy_schema_verbatim() {
        let (base_url, request_rx, handle) = spawn_single_request_server(
            "HTTP/1.1 200 OK",
            r#"{"candidates":[{"content":{"parts":[{"text":"{\"categories\":[[\"Speech Recognition\"],[\"Speech Recognition\",\"Acoustic Modelling\"]]}"}]}}]}"#,
        );
        let client = GeminiClient::new(
            "models/gemini-2.5-flash".to_string(),
            Some(base_url),
            Some("test-key".to_string()),
        );
        let schema = flat_taxonomy_schema();

        client
            .chat_json("taxonomy system", "taxonomy user", &schema)
            .await
            .expect("request should succeed");

        let request = request_rx.recv().expect("captured request");
        let request = String::from_utf8(request).expect("request should be utf8");
        let body = request_body_json(&request);

        assert_eq!(
            body["generationConfig"]["responseSchema"],
            gemini_response_schema(schema.schema())
        );
        assert_eq!(
            body["generationConfig"]["responseSchema"]["properties"]["categories"]["items"]["type"],
            json!("array")
        );
        assert_eq!(
            body["generationConfig"]["responseSchema"]["properties"]["categories"]["items"]["items"]
                ["type"],
            json!("string")
        );
        assert!(
            request
                .starts_with("POST /models/gemini-2.5-flash:generateContent?key=test-key HTTP/1.1")
        );

        handle.join().expect("server thread");
    }

    #[tokio::test]
    async fn chat_json_sends_project_taxonomy_prompt_and_schema_to_gemini() {
        let (base_url, request_rx, handle) = spawn_single_request_server(
            "HTTP/1.1 200 OK",
            r#"{
                "candidates":[{"content":{"parts":[{"text":"{\"categories\":[[\"Speech Recognition\"],[\"Speech Recognition\",\"Acoustic Modelling\"]]}"}]}}]
            }"#,
        );
        let client = GeminiClient::new(
            "gemini-2.5-flash".to_string(),
            Some(base_url),
            Some("test-key".to_string()),
        );
        let schema = flat_taxonomy_schema();
        let system =
            "You design hierarchical folder taxonomies for academic PDFs. Return strict JSON only.";
        let user = "Return JSON with schema:\n{\"categories\":[[\"Top Level\"],[\"Top Level\",\"Subcategory\"]]}\nRules:\n- each entry in `categories` must be a full category path from root to a category node\n- include parent paths before child paths\n- do not compress the taxonomy just to satisfy an artificial depth target; the final merge stage will handle any depth reduction\n- names must be filesystem-friendly (letters, numbers, spaces, dashes)\n- avoid duplicate category paths\n- output at least one top-level category path\n\nkeywords:\n[\"acoustic modelling\",\"connectionist temporal classification\",\"deep recurrent neural networks\",\"hidden markov models\",\"long short term memory\",\"lstm\",\"sequence labelling\",\"speech recognition\"]";

        let response = client
            .chat_json(system, user, &schema)
            .await
            .expect("request should succeed");
        println!("taxonomy adapter test response: {}", response.content);

        let parsed: Value =
            serde_json::from_str(&response.content).expect("response content should be valid json");
        assert_eq!(parsed["categories"][0], json!(["Speech Recognition"]));
        assert_eq!(
            parsed["categories"][1],
            json!(["Speech Recognition", "Acoustic Modelling"])
        );
        assert!(
            parsed["categories"]
                .as_array()
                .is_some_and(|categories| !categories.is_empty()),
            "taxonomy response should contain at least one top-level category"
        );

        let request = request_rx.recv().expect("captured request");
        let request = String::from_utf8(request).expect("request should be utf8");
        let body = request_body_json(&request);

        assert!(body["systemInstruction"].is_null());
        assert_eq!(
            body["contents"][0]["parts"][0]["text"],
            format!("{system}\n\n{user}")
        );
        assert_eq!(
            body["generationConfig"]["responseMimeType"],
            Value::String("application/json".to_string())
        );
        assert_eq!(
            body["generationConfig"]["responseSchema"],
            gemini_response_schema(schema.schema())
        );
        assert_eq!(
            body["generationConfig"]["responseSchema"]["properties"]["categories"]["items"]["type"],
            json!("array")
        );
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "MEDIUM"
        );
        assert!(body["generationConfig"]["temperature"].is_null());

        handle.join().expect("server thread");
    }

    #[tokio::test]
    async fn chat_uses_lean_plain_text_request_shape() {
        let (base_url, request_rx, handle) = spawn_single_request_server(
            "HTTP/1.1 200 OK",
            r#"{"candidates":[{"content":{"parts":[{"text":"plain text"}]}}]}"#,
        );
        let client = GeminiClient::new(
            "gemini-2.5-flash".to_string(),
            Some(base_url),
            Some("test-key".to_string()),
        );

        let response = client
            .chat("system prompt", "user prompt")
            .await
            .expect("request should succeed");

        assert_eq!(response.content, "plain text");

        let request = request_rx.recv().expect("captured request");
        let request = String::from_utf8(request).expect("request should be utf8");
        let body = request_body_json(&request);

        assert!(body["generationConfig"]["responseMimeType"].is_null());
        assert!(body["generationConfig"]["responseJsonSchema"].is_null());
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "MEDIUM"
        );
        assert!(body["systemInstruction"].is_null());
        assert_eq!(
            body["contents"][0]["parts"][0]["text"],
            "system prompt\n\nuser prompt"
        );
        assert!(body["generationConfig"]["temperature"].is_null());

        handle.join().expect("server thread");
    }

    #[tokio::test]
    #[ignore = "requires SYP_API_KEY and network access; run explicitly"]
    async fn live_gemini_taxonomy_request_returns_non_empty_categories() {
        let api_key =
            env::var("SYP_API_KEY").expect("set SYP_API_KEY to run the live Gemini adapter test");
        let model = env::var("SYP_LLM_MODEL").unwrap_or_else(|_| "gemini-2.5-flash".to_string());
        let base_url = env::var("SYP_LLM_BASE_URL").ok();

        let client = GeminiClient::new(model, base_url, Some(api_key));
        let schema = flat_taxonomy_schema();
        let system =
            "You design hierarchical folder taxonomies for academic PDFs. Return strict JSON only.";
        let user = "Return JSON with schema:\n{\"categories\":[[\"Top Level\"],[\"Top Level\",\"Subcategory\"]]}\nRules:\n- each entry in `categories` must be a full category path from root to a category node\n- include parent paths before child paths\n- do not compress the taxonomy just to satisfy an artificial depth target; the final merge stage will handle any depth reduction\n- names must be filesystem-friendly (letters, numbers, spaces, dashes)\n- avoid duplicate category paths\n- output at least one top-level category path\n\nkeywords:\n[\"acoustic modelling\",\"connectionist temporal classification\",\"deep recurrent neural networks\",\"hidden markov models\",\"long short term memory\",\"lstm\",\"sequence labelling\",\"speech recognition\"]";

        let response = timeout(
            Duration::from_secs(90),
            client.chat_json(system, user, &schema),
        )
        .await
        .expect("live Gemini request exceeded 90s timeout")
        .expect("live Gemini request should succeed");
        println!("live taxonomy adapter response: {}", response.content);

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
