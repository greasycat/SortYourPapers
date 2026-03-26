use super::*;

pub(super) fn top_score_and_margin(
    ranking: &[PlacementCandidateScore],
) -> (Option<f32>, Option<f32>) {
    let Some(top) = ranking.first() else {
        return (None, None);
    };
    let margin = ranking
        .get(1)
        .map(|runner_up| top.similarity - runner_up.similarity);
    (Some(top.similarity), margin)
}

pub(super) fn weighted_centroid(matches: &[ReferenceMatchRecord]) -> Option<Vec<f32>> {
    let dimensions = matches.first()?.embedding.len();
    if dimensions == 0 {
        return None;
    }

    let mut centroid = vec![0.0_f32; dimensions];
    let mut total_weight = 0.0_f32;
    for record in matches {
        if record.embedding.len() != dimensions || record.similarity <= 0.0 {
            continue;
        }
        total_weight += record.similarity;
        for (value, dimension) in record.embedding.iter().zip(centroid.iter_mut()) {
            *dimension += value * record.similarity;
        }
    }
    if total_weight <= 0.0 {
        return None;
    }
    for dimension in &mut centroid {
        *dimension /= total_weight;
    }
    Some(centroid)
}

pub(super) fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (lhs, rhs) in left.iter().zip(right.iter()) {
        dot += lhs * rhs;
        left_norm += lhs * lhs;
        right_norm += rhs * rhs;
    }
    if left_norm <= 0.0 || right_norm <= 0.0 {
        return 0.0;
    }
    dot / (left_norm.sqrt() * right_norm.sqrt())
}
