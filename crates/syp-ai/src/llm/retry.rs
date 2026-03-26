use serde::de::DeserializeOwned;
use std::time::Duration;
use tokio::time::sleep;

use crate::{
    error::{AppError, Result},
    llm::LlmCallMetrics,
};

use super::{
    client::{LlmClient, LlmResponse, ParsedLlmResponse},
    schema::JsonResponseSchema,
};

const MAX_HTTP_ATTEMPTS: usize = 3;
const HTTP_RETRY_BASE_DELAY_MS: u64 = 500;

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

pub fn strip_code_fence(raw: &str) -> String {
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
