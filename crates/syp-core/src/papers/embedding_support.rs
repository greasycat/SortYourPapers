use async_trait::async_trait;
use paperdb::{
    EmbeddingCallMetrics as DbEmbeddingCallMetrics, EmbeddingClient as DbEmbeddingClient,
    EmbeddingModelId, EmbeddingRequest as DbEmbeddingRequest,
    EmbeddingResponse as DbEmbeddingResponse, EmbeddingVector as DbEmbeddingVector, PaperDbError,
    PaperInput,
};

use crate::{
    error::{AppError, Result},
    llm::{EmbeddingConfig, LlmCallMetrics, LlmProvider, LlmUsageSummary},
    papers::PaperText,
};

pub(crate) struct PaperDbEmbeddingAdapter<'a> {
    pub(crate) inner: &'a dyn crate::llm::EmbeddingClient,
}

#[async_trait]
impl DbEmbeddingClient for PaperDbEmbeddingAdapter<'_> {
    async fn embed(&self, request: &DbEmbeddingRequest) -> paperdb::Result<DbEmbeddingResponse> {
        let response = self
            .inner
            .embed(&crate::llm::EmbeddingRequest::from_texts(
                request.inputs.clone(),
            ))
            .await
            .map_err(|err| PaperDbError::Config(err.to_string()))?;
        Ok(DbEmbeddingResponse {
            embeddings: response
                .embeddings
                .into_iter()
                .map(|vector| DbEmbeddingVector {
                    values: vector.values,
                })
                .collect(),
            metrics: DbEmbeddingCallMetrics {
                provider: response.metrics.provider,
                model: response.metrics.model,
                endpoint_kind: response.metrics.endpoint_kind,
                request_chars: response.metrics.request_chars,
                response_chars: response.metrics.response_chars,
                http_attempt_count: response.metrics.http_attempt_count,
                json_retry_count: response.metrics.json_retry_count,
                semantic_retry_count: response.metrics.semantic_retry_count,
                input_tokens: response.metrics.input_tokens,
                output_tokens: response.metrics.output_tokens,
                total_tokens: response.metrics.total_tokens,
            },
        })
    }
}

#[must_use]
pub(crate) fn build_embedding_config(
    provider: LlmProvider,
    model: String,
    base_url: Option<String>,
    api_key: Option<String>,
) -> EmbeddingConfig {
    EmbeddingConfig {
        provider,
        model,
        base_url,
        api_key,
    }
}

#[must_use]
pub(crate) fn embedding_model_id(
    provider: LlmProvider,
    model: impl Into<String>,
) -> EmbeddingModelId {
    EmbeddingModelId::new(provider_name(provider), model.into())
}

#[must_use]
pub(crate) fn paper_input_from(paper: &PaperText) -> PaperInput {
    PaperInput {
        file_id: paper.file_id.clone(),
        source_path: paper.path.clone(),
        extracted_text: paper.extracted_text.clone(),
        llm_ready_text: paper.llm_ready_text.clone(),
        pages_read: paper.pages_read,
    }
}

#[must_use]
pub(crate) fn usage_from_db_metrics(metrics: Option<&DbEmbeddingCallMetrics>) -> LlmUsageSummary {
    let Some(metrics) = metrics else {
        return LlmUsageSummary::default();
    };

    let mut usage = LlmUsageSummary::default();
    usage.record_call(&LlmCallMetrics {
        provider: metrics.provider.clone(),
        model: metrics.model.clone(),
        endpoint_kind: metrics.endpoint_kind.clone(),
        request_chars: metrics.request_chars,
        response_chars: metrics.response_chars,
        http_attempt_count: metrics.http_attempt_count,
        json_retry_count: metrics.json_retry_count,
        semantic_retry_count: metrics.semantic_retry_count,
        input_tokens: metrics.input_tokens,
        output_tokens: metrics.output_tokens,
        total_tokens: metrics.total_tokens,
    });
    usage
}

pub(crate) fn map_paperdb_error(err: PaperDbError) -> AppError {
    AppError::Execution(format!("paperdb error: {err}"))
}

fn provider_name(provider: LlmProvider) -> &'static str {
    match provider {
        LlmProvider::Openai => "openai",
        LlmProvider::Ollama => "ollama",
        LlmProvider::Gemini => "gemini",
    }
}

pub(crate) fn embedding_config_from_app(
    config: &crate::config::AppConfig,
) -> Result<EmbeddingConfig> {
    Ok(build_embedding_config(
        config.embedding_provider,
        config.embedding_model.clone(),
        config.embedding_base_url.clone(),
        config.resolved_embedding_api_key()?,
    ))
}
