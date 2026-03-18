mod batch;
mod client;
pub mod providers;
mod retry;
mod schema;

pub use batch::{
    DEFAULT_REQUEST_DISPATCH_DELAY_MS, RequestBatchOptions, run_delayed_concurrent_requests,
};
pub(crate) use batch::{batch_dispatch_spacing, wait_for_dispatch_slot};
pub use client::{LlmClient, LlmResponse, ParsedLlmResponse, build_client};
pub use providers::{gemini, ollama, openai};
pub(crate) use retry::strip_code_fence;
pub use retry::{call_json_with_retry, call_text_with_retry};
pub use schema::JsonResponseSchema;

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
