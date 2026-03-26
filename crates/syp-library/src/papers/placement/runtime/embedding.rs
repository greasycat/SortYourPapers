use super::*;

pub(super) async fn prepare_embedding_runtime(
    papers: &[PaperText],
    allowed_targets: &[String],
    options: &PlacementEmbeddingOptions,
    verbosity: crate::terminal::Verbosity,
) -> Result<(PlacementEmbeddingRuntime, LlmUsageSummary)> {
    let db = PaperDb::open_default().map_err(map_paperdb_error)?;
    let model_id = embedding_model_id(options.provider, options.model.clone());
    let manifest = load_manifest_from_path(&options.reference_manifest_path)?;
    let reference_set = ReferenceSetInput {
        set_id: manifest.set_id.clone(),
        manifest_path: options.reference_manifest_path.clone(),
        manifest_fingerprint: manifest_fingerprint(&options.reference_manifest_path)?,
        papers: manifest
            .papers
            .iter()
            .map(reference_paper_input_from)
            .collect(),
    };
    let embedding_client = build_embedding_client(&build_embedding_config(
        options.provider,
        options.model.clone(),
        options.base_url.clone(),
        options.api_key.clone(),
    ))?;
    let adapter = PaperDbEmbeddingAdapter {
        inner: embedding_client.as_ref(),
    };

    let mut usage = LlmUsageSummary::default();
    let reference_sync = db
        .sync_reference_set(&reference_set, &adapter, &model_id, false)
        .await
        .map_err(map_paperdb_error)?;
    usage.merge(&usage_from_db_metrics(reference_sync.metrics.as_ref()));
    verbosity.stage_line(
        "placements",
        format!(
            "{} reference index for placement from {} with {} paper(s)",
            if reference_sync.skipped {
                "reused"
            } else {
                "updated"
            },
            manifest.set_id,
            reference_sync.papers_indexed
        ),
    );

    let paper_inputs = papers.iter().map(paper_input_from).collect::<Vec<_>>();
    let paper_sync = db
        .sync_embeddings(&paper_inputs, &adapter, &model_id)
        .await
        .map_err(map_paperdb_error)?;
    usage.merge(&usage_from_db_metrics(paper_sync.metrics.as_ref()));
    verbosity.stage_line(
        "placements",
        format!(
            "placement retrieval synced {} paper embedding(s), skipped {} unchanged row(s)",
            paper_sync.embeddings_upserted, paper_sync.embeddings_skipped
        ),
    );

    let paper_embeddings = papers
        .iter()
        .filter_map(|paper| {
            db.get_embedding(&paper.file_id, &model_id.provider, &model_id.model)
                .map_err(map_paperdb_error)
                .transpose()
                .map(|record| record.map(|record| (paper.file_id.clone(), record.embedding)))
        })
        .collect::<Result<HashMap<_, _>>>()?;

    let target_query_texts = allowed_targets
        .iter()
        .map(|target| target_query_text(target))
        .collect::<Vec<_>>();
    let target_embeddings_response = embedding_client
        .embed(&crate::llm::EmbeddingRequest::from_texts(
            target_query_texts.clone(),
        ))
        .await?;
    usage.record_call(&target_embeddings_response.metrics);

    let mut target_profiles = Vec::with_capacity(allowed_targets.len());
    let mut target_embeddings = HashMap::with_capacity(allowed_targets.len());

    for ((target_rel_path, query_text), query_embedding) in allowed_targets
        .iter()
        .cloned()
        .zip(target_query_texts.into_iter())
        .zip(target_embeddings_response.embeddings.into_iter())
    {
        let matches = db
            .nearest_reference_matches(&model_id, &query_embedding.values, options.reference_top_k)
            .map_err(map_paperdb_error)?;
        let strong_matches = matches
            .iter()
            .filter(|candidate| candidate.similarity >= options.min_similarity)
            .cloned()
            .collect::<Vec<_>>();
        let (source, centroid_embedding, reference_support) =
            if strong_matches.len() >= options.min_reference_support {
                (
                    PlacementTargetProfileSource::ReferenceCentroid,
                    scoring::weighted_centroid(&strong_matches)
                        .unwrap_or_else(|| query_embedding.values.clone()),
                    strong_matches
                        .iter()
                        .map(reference_support_from)
                        .collect::<Vec<_>>(),
                )
            } else {
                (
                    PlacementTargetProfileSource::TargetPathEmbedding,
                    query_embedding.values.clone(),
                    strong_matches
                        .iter()
                        .map(reference_support_from)
                        .collect::<Vec<_>>(),
                )
            };
        target_profiles.push(PlacementTargetProfile {
            target_rel_path: target_rel_path.clone(),
            query_text,
            source,
            reference_support_count: reference_support.len(),
            reference_support,
        });
        target_embeddings.insert(target_rel_path, centroid_embedding);
    }

    Ok((
        PlacementEmbeddingRuntime {
            allowed_targets: allowed_targets.to_vec(),
            target_profiles,
            target_embeddings,
            paper_embeddings,
            candidate_top_k: options.candidate_top_k,
            min_similarity: options.min_similarity,
            min_margin: options.min_margin,
        },
        usage,
    ))
}

pub(super) fn rank_targets_for_paper(
    paper: &PaperText,
    runtime: &PlacementEmbeddingRuntime,
) -> Result<Vec<PlacementCandidateScore>> {
    let paper_embedding = runtime
        .paper_embeddings
        .get(&paper.file_id)
        .ok_or_else(|| {
            AppError::Validation(format!(
                "missing stored embedding for placement paper {}",
                paper.file_id
            ))
        })?;

    let mut ranking = runtime
        .allowed_targets
        .iter()
        .filter_map(|target| {
            runtime
                .target_embeddings
                .get(target)
                .map(|target_embedding| PlacementCandidateScore {
                    target_rel_path: target.clone(),
                    similarity: scoring::cosine_similarity(paper_embedding, target_embedding),
                })
        })
        .collect::<Vec<_>>();
    ranking.sort_by(|left, right| {
        right
            .similarity
            .partial_cmp(&left.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.target_rel_path.cmp(&right.target_rel_path))
    });
    Ok(ranking)
}

pub(super) fn should_use_embedding_decision(
    ranking: &[PlacementCandidateScore],
    runtime: &PlacementEmbeddingRuntime,
) -> bool {
    let Some(top) = ranking.first() else {
        return false;
    };
    if ranking.len() == 1 {
        return true;
    }
    let margin = ranking
        .get(1)
        .map(|runner_up| top.similarity - runner_up.similarity)
        .unwrap_or(f32::INFINITY);
    top.similarity >= runtime.min_similarity && margin >= runtime.min_margin
}

fn target_query_text(target_rel_path: &str) -> String {
    if target_rel_path == "." {
        DEFAULT_ROOT_TARGET_TEXT.to_string()
    } else {
        format!("category path: {}", target_rel_path.replace('/', " / "))
    }
}

fn reference_support_from(record: &ReferenceMatchRecord) -> PlacementReferenceSupport {
    PlacementReferenceSupport {
        paper_id: record.paper_id.clone(),
        title: record.title.clone(),
        similarity: record.similarity,
    }
}

fn reference_paper_input_from(paper: &CuratedPaper) -> ReferencePaperInput {
    ReferencePaperInput {
        paper_id: paper.paper_id.clone(),
        title: paper.title.clone(),
        category: paper.category.clone(),
        subcategory: paper.subcategory.clone(),
        abstract_excerpt: paper.abstract_excerpt.clone(),
        embedding_text: format!(
            "title: {}\ncategory: {}\nsubcategory: {}\nabstract: {}",
            paper.title, paper.category, paper.subcategory, paper.abstract_excerpt
        ),
    }
}

fn manifest_fingerprint(path: &std::path::Path) -> Result<String> {
    let raw = fs::read(path)?;
    let mut hasher = DefaultHasher::new();
    raw.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}
