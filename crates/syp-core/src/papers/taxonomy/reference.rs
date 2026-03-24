use std::{
    collections::HashMap,
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use paperdb::{
    EmbeddingCallMetrics as DbEmbeddingCallMetrics, EmbeddingClient as DbEmbeddingClient,
    EmbeddingModelId, EmbeddingRequest as DbEmbeddingRequest,
    EmbeddingResponse as DbEmbeddingResponse, EmbeddingVector as DbEmbeddingVector, PaperDb,
    PaperDbError, PaperInput, ReferencePaperInput, ReferenceSetInput,
};

use crate::{
    config::AppConfig,
    error::{AppError, Result},
    llm::{EmbeddingConfig, LlmUsageSummary, build_embedding_client},
    papers::PaperText,
    papers::taxonomy::{ReferenceExemplar, ReferenceLabelScore, TaxonomyReferenceEvidence},
    terminal::Verbosity,
    testsets::{CuratedPaper, CuratedTestSet, load_manifest_from_path},
};

const MAX_REFERENCE_LABELS: usize = 10;
const MAX_REFERENCE_EXEMPLARS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReferenceIndexReport {
    pub(crate) db_path: PathBuf,
    pub(crate) set_id: String,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) papers_indexed: usize,
    pub(crate) skipped: bool,
}

pub(crate) async fn collect_reference_evidence(
    papers: &[PaperText],
    config: &AppConfig,
    verbosity: Verbosity,
) -> Result<(Option<TaxonomyReferenceEvidence>, LlmUsageSummary)> {
    if config.taxonomy_assistance != crate::papers::taxonomy::TaxonomyAssistance::EmbeddingGuided {
        return Ok((None, LlmUsageSummary::default()));
    }
    if papers.is_empty() {
        return Ok((None, LlmUsageSummary::default()));
    }

    let mut usage = LlmUsageSummary::default();
    let db = PaperDb::open_default().map_err(map_paperdb_error)?;
    let model_id = embedding_model_id(config);
    let manifest = load_manifest_from_path(&config.reference_manifest_path)?;
    let client = build_embedding_client(&embedding_config(config)?)?;
    let adapter = PaperDbEmbeddingAdapter {
        inner: client.as_ref(),
    };
    let reference_set = build_reference_set_input(&manifest, &config.reference_manifest_path)?;
    let index_result = db
        .sync_reference_set(&reference_set, &adapter, &model_id, false)
        .await
        .map_err(map_paperdb_error)?;
    usage.merge(&usage_from_db_metrics(index_result.metrics.as_ref()));
    if index_result.skipped {
        verbosity.stage_line(
            "taxonomy",
            format!(
                "reference index ready from {} with {} paper(s)",
                manifest.set_id, index_result.papers_indexed
            ),
        );
    } else {
        verbosity.stage_line(
            "taxonomy",
            format!(
                "indexed {} reference paper(s) from {} using {}:{}",
                index_result.papers_indexed, manifest.set_id, model_id.provider, model_id.model
            ),
        );
    }

    let paper_inputs = papers.iter().map(paper_input_from).collect::<Vec<_>>();
    let paper_sync = db
        .sync_embeddings(&paper_inputs, &adapter, &model_id)
        .await
        .map_err(map_paperdb_error)?;
    usage.merge(&usage_from_db_metrics(paper_sync.metrics.as_ref()));
    verbosity.stage_line(
        "taxonomy",
        format!(
            "reference retrieval synced {} paper embedding(s), skipped {} unchanged row(s)",
            paper_sync.embeddings_upserted, paper_sync.embeddings_skipped
        ),
    );

    let mut category_weights = HashMap::<String, f32>::new();
    let mut subcategory_weights = HashMap::<String, f32>::new();
    let mut exemplars = HashMap::<String, ReferenceExemplar>::new();
    let mut matched_papers = 0_usize;

    for paper in papers {
        let Some(embedding) = db
            .get_embedding(&paper.file_id, &model_id.provider, &model_id.model)
            .map_err(map_paperdb_error)?
        else {
            continue;
        };

        let matches = db
            .nearest_reference_matches(&model_id, &embedding.embedding, config.reference_top_k)
            .map_err(map_paperdb_error)?;
        if matches.is_empty() {
            continue;
        }
        matched_papers += 1;

        for reference in matches {
            *category_weights
                .entry(reference.category.clone())
                .or_default() += reference.similarity;
            for token in reference
                .subcategory
                .split_whitespace()
                .filter_map(normalize_subcategory_token)
            {
                *subcategory_weights.entry(token).or_default() += reference.similarity;
            }

            let candidate = ReferenceExemplar {
                paper_id: reference.paper_id.clone(),
                title: reference.title.clone(),
                category: reference.category.clone(),
                subcategory: reference.subcategory.clone(),
                similarity: reference.similarity,
            };
            exemplars
                .entry(reference.paper_id)
                .and_modify(|existing| {
                    if candidate.similarity > existing.similarity {
                        *existing = candidate.clone();
                    }
                })
                .or_insert(candidate);
        }
    }

    if matched_papers == 0 {
        return Ok((None, usage));
    }

    Ok((
        Some(TaxonomyReferenceEvidence {
            set_id: manifest.set_id,
            query_paper_count: matched_papers,
            top_k_per_paper: config.reference_top_k,
            top_categories: top_weighted_labels(category_weights),
            top_subcategory_tokens: top_weighted_labels(subcategory_weights),
            exemplar_matches: top_exemplars(exemplars),
        }),
        usage,
    ))
}

pub(crate) async fn index_reference_manifest(
    config: &AppConfig,
    manifest_path: Option<PathBuf>,
    force: bool,
    verbosity: Verbosity,
) -> Result<ReferenceIndexReport> {
    let manifest_path = manifest_path.unwrap_or_else(|| config.reference_manifest_path.clone());
    let manifest = load_manifest_from_path(&manifest_path)?;
    let reference_set = build_reference_set_input(&manifest, &manifest_path)?;
    let db_path = PaperDb::default_path().map_err(map_paperdb_error)?;
    let db = PaperDb::open(&db_path).map_err(map_paperdb_error)?;
    let model_id = embedding_model_id(config);
    let client = build_embedding_client(&embedding_config(config)?)?;
    let adapter = PaperDbEmbeddingAdapter {
        inner: client.as_ref(),
    };
    let result = db
        .sync_reference_set(&reference_set, &adapter, &model_id, force)
        .await
        .map_err(map_paperdb_error)?;

    usage_from_db_metrics(result.metrics.as_ref());
    verbosity.stage_line(
        "reference-index",
        format!(
            "{} reference index for {} at {} using {}:{}",
            if result.skipped { "reused" } else { "updated" },
            manifest.set_id,
            db_path.display(),
            model_id.provider,
            model_id.model
        ),
    );

    Ok(ReferenceIndexReport {
        db_path,
        set_id: manifest.set_id,
        provider: model_id.provider,
        model: model_id.model,
        papers_indexed: result.papers_indexed,
        skipped: result.skipped,
    })
}

struct PaperDbEmbeddingAdapter<'a> {
    inner: &'a dyn crate::llm::EmbeddingClient,
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

fn embedding_config(config: &AppConfig) -> Result<EmbeddingConfig> {
    Ok(EmbeddingConfig {
        provider: config.embedding_provider,
        model: config.embedding_model.clone(),
        base_url: config.embedding_base_url.clone(),
        api_key: config.resolved_embedding_api_key()?,
    })
}

fn embedding_model_id(config: &AppConfig) -> EmbeddingModelId {
    EmbeddingModelId::new(
        provider_name(config.embedding_provider),
        config.embedding_model.clone(),
    )
}

fn provider_name(provider: crate::llm::LlmProvider) -> &'static str {
    match provider {
        crate::llm::LlmProvider::Openai => "openai",
        crate::llm::LlmProvider::Ollama => "ollama",
        crate::llm::LlmProvider::Gemini => "gemini",
    }
}

fn paper_input_from(paper: &PaperText) -> PaperInput {
    PaperInput {
        file_id: paper.file_id.clone(),
        source_path: paper.path.clone(),
        extracted_text: paper.extracted_text.clone(),
        llm_ready_text: paper.llm_ready_text.clone(),
        pages_read: paper.pages_read,
    }
}

fn build_reference_set_input(
    manifest: &CuratedTestSet,
    manifest_path: &Path,
) -> Result<ReferenceSetInput> {
    Ok(ReferenceSetInput {
        set_id: manifest.set_id.clone(),
        manifest_path: manifest_path.to_path_buf(),
        manifest_fingerprint: manifest_fingerprint(manifest_path)?,
        papers: manifest
            .papers
            .iter()
            .map(reference_paper_input_from)
            .collect(),
    })
}

fn reference_paper_input_from(paper: &CuratedPaper) -> ReferencePaperInput {
    ReferencePaperInput {
        paper_id: paper.paper_id.clone(),
        title: paper.title.clone(),
        category: paper.category.clone(),
        subcategory: paper.subcategory.clone(),
        abstract_excerpt: paper.abstract_excerpt.clone(),
        embedding_text: reference_embedding_text(paper),
    }
}

fn reference_embedding_text(paper: &CuratedPaper) -> String {
    format!(
        "title: {}\ncategory: {}\nsubcategory: {}\nabstract: {}",
        paper.title, paper.category, paper.subcategory, paper.abstract_excerpt
    )
}

fn manifest_fingerprint(path: &Path) -> Result<String> {
    let raw = fs::read(path)?;
    let mut hasher = DefaultHasher::new();
    raw.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

fn normalize_subcategory_token(token: &str) -> Option<String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn top_weighted_labels(weights: HashMap<String, f32>) -> Vec<ReferenceLabelScore> {
    let mut items = weights
        .into_iter()
        .map(|(label, weight)| ReferenceLabelScore { label, weight })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        right
            .weight
            .partial_cmp(&left.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.label.cmp(&right.label))
    });
    items.truncate(MAX_REFERENCE_LABELS);
    items
}

fn top_exemplars(exemplars: HashMap<String, ReferenceExemplar>) -> Vec<ReferenceExemplar> {
    let mut items = exemplars.into_values().collect::<Vec<_>>();
    items.sort_by(|left, right| {
        right
            .similarity
            .partial_cmp(&left.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.paper_id.cmp(&right.paper_id))
    });
    items.truncate(MAX_REFERENCE_EXEMPLARS);
    items
}

fn usage_from_db_metrics(metrics: Option<&DbEmbeddingCallMetrics>) -> LlmUsageSummary {
    let Some(metrics) = metrics else {
        return LlmUsageSummary::default();
    };

    let mut usage = LlmUsageSummary::default();
    usage.record_call(&crate::llm::LlmCallMetrics {
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

fn map_paperdb_error(err: PaperDbError) -> AppError {
    AppError::Execution(format!("paperdb error: {err}"))
}
