use serde::{Deserialize, Serialize};

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
