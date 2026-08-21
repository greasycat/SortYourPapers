//! Reading placement thresholds off a run instead of guessing them.
//!
//! Every run records the score each paper got against its best folder and how
//! far ahead that folder was. Those two distributions are what the placement
//! gates are supposed to cut, so the gates should be chosen from them rather
//! than from round numbers picked before any data existed.

use crate::papers::placement::{PlacementDecisionSource, PlacementEvidence};

/// What a run's placement scores looked like, and what gates they imply.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationReport {
    pub decided_papers: usize,
    pub decided_by_embedding: usize,
    pub decided_by_model: usize,
    pub top_score: Distribution,
    pub margin: Distribution,
    /// Gates that would have sent [`Self::deferral_target`] of decisions to the
    /// model.
    pub suggested_min_similarity: f32,
    pub suggested_min_margin: f32,
    /// Share of decisions the suggestion deliberately hands to the model.
    pub deferral_target: f32,
}

/// Percentiles of one measurement across a run.
#[derive(Debug, Clone, PartialEq)]
pub struct Distribution {
    pub samples: usize,
    pub min: f32,
    pub p10: f32,
    pub median: f32,
    pub p90: f32,
    pub max: f32,
}

/// Share of decisions worth handing to the model rather than settling on
/// embeddings alone.
///
/// The weakest fifth is where embedding rankings are least separated, and it is
/// also where a model call buys the most.
const DEFERRAL_TARGET: f32 = 0.20;

impl CalibrationReport {
    #[must_use]
    pub fn summary_lines(&self) -> Vec<String> {
        vec![
            format!(
                "placement decisions {} | {} by embedding | {} by model",
                self.decided_papers, self.decided_by_embedding, self.decided_by_model
            ),
            format!("top score {}", self.top_score.describe()),
            format!("margin    {}", self.margin.describe()),
            format!(
                "gates that would defer the weakest {:.0}%: min-similarity {:.3}, min-margin {:.3}",
                self.deferral_target * 100.0,
                self.suggested_min_similarity,
                self.suggested_min_margin
            ),
        ]
    }
}

impl Distribution {
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "min {:.3} | p10 {:.3} | median {:.3} | p90 {:.3} | max {:.3} ({} sample(s))",
            self.min, self.p10, self.median, self.p90, self.max, self.samples
        )
    }
}

/// Summarizes the placement scores a run recorded.
///
/// Returns `None` when the run scored nothing, which is the case for a run that
/// placed everything with the model.
#[must_use]
pub fn calibrate(evidence: &PlacementEvidence) -> Option<CalibrationReport> {
    let scores = collect(evidence.papers.iter().filter_map(|paper| paper.top_score));
    let margins = collect(
        evidence
            .papers
            .iter()
            .filter_map(|paper| paper.margin_over_runner_up),
    );

    let top_score = describe(&scores)?;
    // A single candidate has no runner-up, so margins can be scarcer than
    // scores; fall back to the score spread rather than inventing one.
    let margin = describe(&margins).unwrap_or_else(|| top_score.clone());

    let decided_by_embedding = evidence
        .papers
        .iter()
        .filter(|paper| matches!(paper.decision_source, PlacementDecisionSource::Embedding))
        .count();

    Some(CalibrationReport {
        decided_papers: evidence.papers.len(),
        decided_by_embedding,
        decided_by_model: evidence.papers.len() - decided_by_embedding,
        suggested_min_similarity: quantile(&scores, DEFERRAL_TARGET),
        suggested_min_margin: quantile(&margins, DEFERRAL_TARGET),
        top_score,
        margin,
        deferral_target: DEFERRAL_TARGET,
    })
}

fn collect(values: impl Iterator<Item = f32>) -> Vec<f32> {
    let mut values = values.filter(|value| value.is_finite()).collect::<Vec<_>>();
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    values
}

fn describe(sorted: &[f32]) -> Option<Distribution> {
    if sorted.is_empty() {
        return None;
    }
    Some(Distribution {
        samples: sorted.len(),
        min: sorted[0],
        p10: quantile(sorted, 0.10),
        median: quantile(sorted, 0.50),
        p90: quantile(sorted, 0.90),
        max: sorted[sorted.len() - 1],
    })
}

/// Nearest-rank quantile of an already sorted slice.
fn quantile(sorted: &[f32], fraction: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (fraction * sorted.len() as f32).floor() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use crate::papers::placement::{PaperPlacementEvidence, PlacementAssistance};

    use super::*;

    fn paper(
        top_score: f32,
        margin: f32,
        decision_source: PlacementDecisionSource,
    ) -> PaperPlacementEvidence {
        PaperPlacementEvidence {
            file_id: format!("paper-{top_score}-{margin}"),
            chosen_target_rel_path: "AI/Vision".to_string(),
            decision_source,
            top_candidates: Vec::new(),
            top_score: Some(top_score),
            margin_over_runner_up: Some(margin),
        }
    }

    fn evidence(papers: Vec<PaperPlacementEvidence>) -> PlacementEvidence {
        PlacementEvidence {
            assistance: PlacementAssistance::EmbeddingPrimary,
            target_profiles: Vec::new(),
            papers,
        }
    }

    #[test]
    fn a_run_with_no_scores_has_nothing_to_calibrate() {
        assert!(calibrate(&evidence(Vec::new())).is_none());
    }

    #[test]
    fn the_report_separates_embedding_decisions_from_model_decisions() {
        let report = calibrate(&evidence(vec![
            paper(0.8, 0.3, PlacementDecisionSource::Embedding),
            paper(0.7, 0.2, PlacementDecisionSource::Embedding),
            paper(0.4, 0.01, PlacementDecisionSource::LlmTiebreak),
        ]))
        .expect("report");

        assert_eq!(report.decided_papers, 3);
        assert_eq!(report.decided_by_embedding, 2);
        assert_eq!(report.decided_by_model, 1);
    }

    #[test]
    fn suggested_gates_sit_inside_the_observed_spread() {
        let report = calibrate(&evidence(
            (0..10)
                .map(|index| {
                    paper(
                        0.30 + index as f32 * 0.05,
                        0.01 + index as f32 * 0.02,
                        PlacementDecisionSource::Embedding,
                    )
                })
                .collect(),
        ))
        .expect("report");

        assert!(
            report.suggested_min_similarity >= report.top_score.min
                && report.suggested_min_similarity <= report.top_score.max,
            "{report:?}"
        );
        assert!(
            report.suggested_min_similarity > 0.20,
            "measured scores should move the gate off the shipped round number: {report:?}"
        );
        assert!(
            report.suggested_min_margin >= report.margin.min,
            "{report:?}"
        );
    }

    #[test]
    fn the_distribution_reports_the_real_spread() {
        let report = calibrate(&evidence(vec![
            paper(0.10, 0.01, PlacementDecisionSource::Embedding),
            paper(0.50, 0.05, PlacementDecisionSource::Embedding),
            paper(0.90, 0.09, PlacementDecisionSource::Embedding),
        ]))
        .expect("report");

        assert!((report.top_score.min - 0.10).abs() < 1e-6);
        assert!((report.top_score.max - 0.90).abs() < 1e-6);
        assert!((report.top_score.median - 0.50).abs() < 1e-6);
        assert_eq!(report.top_score.samples, 3);
    }
}
