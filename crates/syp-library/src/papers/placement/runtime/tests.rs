use super::*;

#[test]
fn weighted_centroid_uses_similarity_weights() {
    let centroid = scoring::weighted_centroid(&[
        ReferenceMatchRecord {
            set_id: "demo".to_string(),
            paper_id: "p1".to_string(),
            title: "Paper One".to_string(),
            category: "AI".to_string(),
            subcategory: "Vision".to_string(),
            abstract_excerpt: String::new(),
            embedding: vec![1.0, 0.0],
            similarity: 0.75,
        },
        ReferenceMatchRecord {
            set_id: "demo".to_string(),
            paper_id: "p2".to_string(),
            title: "Paper Two".to_string(),
            category: "AI".to_string(),
            subcategory: "Vision".to_string(),
            abstract_excerpt: String::new(),
            embedding: vec![0.0, 1.0],
            similarity: 0.25,
        },
    ])
    .expect("centroid");

    assert!((centroid[0] - 0.75).abs() < 1e-6);
    assert!((centroid[1] - 0.25).abs() < 1e-6);
}

#[test]
fn embedding_decision_requires_similarity_and_margin() {
    let runtime = PlacementEmbeddingRuntime {
        allowed_targets: vec!["AI/Vision".to_string(), "AI/NLP".to_string()],
        target_profiles: Vec::new(),
        target_embeddings: HashMap::new(),
        paper_embeddings: HashMap::new(),
        candidate_top_k: 3,
        min_similarity: 0.20,
        min_margin: 0.05,
    };

    assert!(embedding::should_use_embedding_decision(
        &[
            PlacementCandidateScore {
                target_rel_path: "AI/Vision".to_string(),
                similarity: 0.80,
            },
            PlacementCandidateScore {
                target_rel_path: "AI/NLP".to_string(),
                similarity: 0.50,
            },
        ],
        &runtime,
    ));
    assert!(!embedding::should_use_embedding_decision(
        &[
            PlacementCandidateScore {
                target_rel_path: "AI/Vision".to_string(),
                similarity: 0.18,
            },
            PlacementCandidateScore {
                target_rel_path: "AI/NLP".to_string(),
                similarity: 0.05,
            },
        ],
        &runtime,
    ));
    assert!(!embedding::should_use_embedding_decision(
        &[
            PlacementCandidateScore {
                target_rel_path: "AI/Vision".to_string(),
                similarity: 0.30,
            },
            PlacementCandidateScore {
                target_rel_path: "AI/NLP".to_string(),
                similarity: 0.27,
            },
        ],
        &runtime,
    ));
}
