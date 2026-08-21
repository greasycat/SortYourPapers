use std::collections::HashMap;

use super::*;
use crate::papers::placement::{
    PlacementCandidateScore, PlacementTargetProfile, PlacementTargetProfileSource,
};

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
    // Both targets are reference-backed, so only the scores are under test.
    let runtime = runtime_with(
        vec![
            profile("AI/Vision", PlacementTargetProfileSource::ReferenceCentroid),
            profile("AI/NLP", PlacementTargetProfileSource::ReferenceCentroid),
        ],
        0.20,
        0.05,
    );

    assert!(embedding::should_use_embedding_decision(
        &[candidate("AI/Vision", 0.80), candidate("AI/NLP", 0.50)],
        &runtime,
    ));
    assert!(
        !embedding::should_use_embedding_decision(
            &[candidate("AI/Vision", 0.18), candidate("AI/NLP", 0.05)],
            &runtime,
        ),
        "below the similarity gate"
    );
    assert!(
        !embedding::should_use_embedding_decision(
            &[candidate("AI/Vision", 0.30), candidate("AI/NLP", 0.27)],
            &runtime,
        ),
        "below the margin gate"
    );
}

fn runtime_with(
    profiles: Vec<PlacementTargetProfile>,
    min_similarity: f32,
    min_margin: f32,
) -> PlacementEmbeddingRuntime {
    PlacementEmbeddingRuntime {
        allowed_targets: profiles
            .iter()
            .map(|profile| profile.target_rel_path.clone())
            .collect(),
        target_profiles: profiles,
        target_embeddings: HashMap::new(),
        paper_embeddings: HashMap::new(),
        candidate_top_k: 3,
        min_similarity,
        min_margin,
    }
}

fn profile(target: &str, source: PlacementTargetProfileSource) -> PlacementTargetProfile {
    PlacementTargetProfile {
        target_rel_path: target.to_string(),
        query_text: target.to_string(),
        source,
        reference_support_count: 0,
        reference_support: Vec::new(),
    }
}

fn candidate(target: &str, similarity: f32) -> PlacementCandidateScore {
    PlacementCandidateScore {
        target_rel_path: target.to_string(),
        similarity,
    }
}

#[test]
fn a_clear_win_over_a_reference_backed_target_is_decided_by_embeddings() {
    let runtime = runtime_with(
        vec![profile(
            "AI/Vision",
            PlacementTargetProfileSource::ReferenceCentroid,
        )],
        0.20,
        0.05,
    );

    let decided = embedding::should_use_embedding_decision(
        &[candidate("AI/Vision", 0.62), candidate("AI/Speech", 0.31)],
        &runtime,
    );

    assert!(decided);
}

#[test]
fn a_target_profiled_only_by_its_folder_name_goes_to_the_model() {
    let runtime = runtime_with(
        vec![profile(
            "AI/Vision",
            PlacementTargetProfileSource::TargetPathEmbedding,
        )],
        0.20,
        0.05,
    );

    let decided = embedding::should_use_embedding_decision(
        &[candidate("AI/Vision", 0.92), candidate("AI/Speech", 0.10)],
        &runtime,
    );

    assert!(
        !decided,
        "a folder-name profile carries no paper evidence, however high it scores"
    );
}

#[test]
fn a_narrow_win_still_goes_to_the_model() {
    let runtime = runtime_with(
        vec![profile(
            "AI/Vision",
            PlacementTargetProfileSource::ReferenceCentroid,
        )],
        0.20,
        0.05,
    );

    let decided = embedding::should_use_embedding_decision(
        &[candidate("AI/Vision", 0.61), candidate("AI/Speech", 0.60)],
        &runtime,
    );

    assert!(!decided, "a 0.01 margin is not a decision");
}

#[test]
fn the_reference_retrieval_bar_is_stricter_than_the_placement_gate() {
    // The placement gate ships at 0.20 (DEFAULT_PLACEMENT_MIN_SIMILARITY in
    // syp-workflow, which this crate cannot import).
    assert!(
        embedding::REFERENCE_MATCH_MIN_SIMILARITY > 0.20,
        "matching a paper to a label name is a harder ask than ranking profiles"
    );
}
