use std::{
    collections::HashMap,
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
};

use paper_db::{PaperDb, ReferencePaperInput, ReferenceSetInput};

use crate::{
    error::Result,
    llm::{LlmUsageSummary, build_embedding_client},
    papers::taxonomy::{ReferenceExemplar, ReferenceLabelScore, TaxonomyReferenceEvidence},
    papers::{
        PaperText,
        embedding_support::{
            PaperDbEmbeddingAdapter, build_embedding_config, embedding_model_id, map_paperdb_error,
            paper_input_from, usage_from_db_metrics,
        },
    },
    terminal::Verbosity,
    testsets::{CuratedPaper, CuratedTestSet, load_manifest_from_path},
};

const MAX_REFERENCE_LABELS: usize = 10;
const MAX_REFERENCE_EXEMPLARS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceIndexReport {
    pub db_path: PathBuf,
    pub set_id: String,
    pub provider: String,
    pub model: String,
    pub papers_indexed: usize,
    pub skipped: bool,
}

#[derive(Debug, Clone)]
pub struct ReferenceEmbeddingOptions {
    pub provider: crate::llm::LlmProvider,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReferenceEvidenceOptions {
    pub assistance: super::TaxonomyAssistance,
    pub reference_manifest_path: PathBuf,
    pub reference_top_k: usize,
    pub embedding: ReferenceEmbeddingOptions,
}

pub async fn collect_reference_evidence(
    papers: &[PaperText],
    options: &ReferenceEvidenceOptions,
    verbosity: Verbosity,
) -> Result<(Option<TaxonomyReferenceEvidence>, LlmUsageSummary)> {
    if options.assistance != crate::papers::taxonomy::TaxonomyAssistance::EmbeddingGuided {
        return Ok((None, LlmUsageSummary::default()));
    }
    if papers.is_empty() {
        return Ok((None, LlmUsageSummary::default()));
    }

    let mut usage = LlmUsageSummary::default();
    let db = PaperDb::open_default().map_err(map_paperdb_error)?;
    let model_id = embedding_model_id(options.embedding.provider, options.embedding.model.clone());
    let manifest = load_manifest_from_path(&options.reference_manifest_path)?;
    let client = build_embedding_client(&build_embedding_config(
        options.embedding.provider,
        options.embedding.model.clone(),
        options.embedding.base_url.clone(),
        options.embedding.api_key.clone(),
    ))?;
    let adapter = PaperDbEmbeddingAdapter {
        inner: client.as_ref(),
    };
    let reference_set = build_reference_set_input(&manifest, &options.reference_manifest_path)?;
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
            .nearest_reference_matches(&model_id, &embedding.embedding, options.reference_top_k)
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
            top_k_per_paper: options.reference_top_k,
            top_categories: top_weighted_labels(category_weights),
            top_subcategory_tokens: top_weighted_labels(subcategory_weights),
            exemplar_matches: top_exemplars(exemplars),
        }),
        usage,
    ))
}

pub async fn index_reference_manifest(
    options: &ReferenceEmbeddingOptions,
    manifest_path: Option<PathBuf>,
    force: bool,
    verbosity: Verbosity,
) -> Result<ReferenceIndexReport> {
    let manifest_path =
        manifest_path.unwrap_or_else(|| PathBuf::from(crate::testsets::DEFAULT_MANIFEST_PATH));
    let manifest = load_manifest_from_path(&manifest_path)?;
    let reference_set = build_reference_set_input(&manifest, &manifest_path)?;
    let db_path = PaperDb::default_path().map_err(map_paperdb_error)?;
    let db = PaperDb::open(&db_path).map_err(map_paperdb_error)?;
    let model_id = embedding_model_id(options.provider, options.model.clone());
    let client = build_embedding_client(&build_embedding_config(
        options.provider,
        options.model.clone(),
        options.base_url.clone(),
        options.api_key.clone(),
    ))?;
    let adapter = PaperDbEmbeddingAdapter {
        inner: client.as_ref(),
    };
    let result = db
        .sync_reference_set(&reference_set, &adapter, &model_id, force)
        .await
        .map_err(map_paperdb_error)?;

    let _ = usage_from_db_metrics(result.metrics.as_ref());
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
