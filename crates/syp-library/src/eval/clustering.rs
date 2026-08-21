//! Scoring a produced folder tree against known labels.
//!
//! The organizer's output is a partition: every paper lands in exactly one
//! folder. A reference label set is another partition of the same papers. All
//! the measures here compare those two partitions, so none of them needs an
//! LLM, a judge, or a human — only the contingency table between them.
//!
//! Cluster names are deliberately never compared to label names. A run that
//! groups every optics paper together scores well whether it called the folder
//! "Optics" or "Wave Physics"; naming is a separate question from grouping.

use std::collections::BTreeMap;

/// One paper's predicted folder and its reference label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterAssignment {
    pub item: String,
    /// Folder the run placed the paper in.
    pub predicted: String,
    /// Reference label, such as an arXiv subcategory.
    pub truth: String,
}

/// How well a run's folders line up with the reference labels.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusteringMetrics {
    pub items: usize,
    pub predicted_clusters: usize,
    pub truth_classes: usize,
    /// Share of papers in the majority label of their folder. Rises with more
    /// folders, so it is only meaningful next to the cluster count.
    pub purity: f64,
    /// 1 when every folder holds a single label.
    pub homogeneity: f64,
    /// 1 when every label sits in a single folder.
    pub completeness: f64,
    /// Harmonic mean of the two above; equals NMI under arithmetic
    /// normalization.
    pub v_measure: f64,
    /// Pair-counting agreement corrected for chance: 0 is random, 1 is exact.
    /// The one measure here that does not reward splitting.
    pub adjusted_rand_index: f64,
}

impl ClusteringMetrics {
    /// One line per metric, for reports and diffs between runs.
    #[must_use]
    pub fn summary_lines(&self) -> Vec<String> {
        vec![
            format!(
                "items {} | folders {} | reference labels {}",
                self.items, self.predicted_clusters, self.truth_classes
            ),
            format!("adjusted rand index {:.3}", self.adjusted_rand_index),
            format!("v-measure          {:.3}", self.v_measure),
            format!("homogeneity        {:.3}", self.homogeneity),
            format!("completeness       {:.3}", self.completeness),
            format!("purity             {:.3}", self.purity),
        ]
    }
}

/// Scores predicted folders against reference labels.
///
/// Returns `None` for an empty assignment list, where no measure is defined.
#[must_use]
pub fn score(assignments: &[ClusterAssignment]) -> Option<ClusteringMetrics> {
    if assignments.is_empty() {
        return None;
    }

    let table = ContingencyTable::build(assignments);
    let total = table.total as f64;

    let homogeneity = table.reduction_in_uncertainty(Axis::Truth);
    let completeness = table.reduction_in_uncertainty(Axis::Predicted);
    let v_measure = harmonic_mean(homogeneity, completeness);

    Some(ClusteringMetrics {
        items: table.total,
        predicted_clusters: table.predicted_totals.len(),
        truth_classes: table.truth_totals.len(),
        purity: table.majority_agreement() / total,
        homogeneity,
        completeness,
        v_measure,
        adjusted_rand_index: table.adjusted_rand_index(),
    })
}

enum Axis {
    Predicted,
    Truth,
}

struct ContingencyTable {
    /// Counts keyed by (predicted folder, reference label).
    cells: BTreeMap<(String, String), usize>,
    predicted_totals: BTreeMap<String, usize>,
    truth_totals: BTreeMap<String, usize>,
    total: usize,
}

impl ContingencyTable {
    fn build(assignments: &[ClusterAssignment]) -> Self {
        let mut cells = BTreeMap::new();
        let mut predicted_totals = BTreeMap::new();
        let mut truth_totals = BTreeMap::new();

        for assignment in assignments {
            *cells
                .entry((assignment.predicted.clone(), assignment.truth.clone()))
                .or_insert(0) += 1;
            *predicted_totals
                .entry(assignment.predicted.clone())
                .or_insert(0) += 1;
            *truth_totals.entry(assignment.truth.clone()).or_insert(0) += 1;
        }

        Self {
            cells,
            predicted_totals,
            truth_totals,
            total: assignments.len(),
        }
    }

    /// Papers sitting in the majority label of their folder.
    fn majority_agreement(&self) -> f64 {
        let mut best_per_cluster = BTreeMap::<&String, usize>::new();
        for ((predicted, _), count) in &self.cells {
            let best = best_per_cluster.entry(predicted).or_insert(0);
            *best = (*best).max(*count);
        }
        best_per_cluster.values().sum::<usize>() as f64
    }

    /// How much of one axis's entropy the other axis explains.
    ///
    /// With `Axis::Truth` this is homogeneity: how much knowing the folder
    /// tells you about the label. Flipping the axis gives completeness.
    fn reduction_in_uncertainty(&self, axis: Axis) -> f64 {
        let (target_totals, condition_totals) = match axis {
            Axis::Truth => (&self.truth_totals, &self.predicted_totals),
            Axis::Predicted => (&self.predicted_totals, &self.truth_totals),
        };

        let target_entropy = entropy(target_totals.values().copied(), self.total);
        if target_entropy <= 0.0 {
            // One label only: knowing the folder cannot tell you less.
            return 1.0;
        }

        let mut conditional = 0.0;
        for ((predicted, truth), count) in &self.cells {
            let (target_key, condition_key) = match axis {
                Axis::Truth => (truth, predicted),
                Axis::Predicted => (predicted, truth),
            };
            let _ = target_key;
            let condition_total = condition_totals
                .get(condition_key)
                .copied()
                .unwrap_or_default();
            if condition_total == 0 || *count == 0 {
                continue;
            }
            let joint = *count as f64 / self.total as f64;
            conditional -= joint * (*count as f64 / condition_total as f64).ln();
        }

        (1.0 - conditional / target_entropy).clamp(0.0, 1.0)
    }

    /// Rand index adjusted for the agreement two random partitions of these
    /// sizes would reach by chance.
    fn adjusted_rand_index(&self) -> f64 {
        let pairs_in_cells: f64 = self.cells.values().map(|count| pairs(*count)).sum();
        let pairs_in_predicted: f64 = self.predicted_totals.values().map(|c| pairs(*c)).sum();
        let pairs_in_truth: f64 = self.truth_totals.values().map(|c| pairs(*c)).sum();
        let total_pairs = pairs(self.total);

        if total_pairs == 0.0 {
            return 0.0;
        }

        let expected = pairs_in_predicted * pairs_in_truth / total_pairs;
        let max = 0.5 * (pairs_in_predicted + pairs_in_truth);
        if (max - expected).abs() < f64::EPSILON {
            // Both partitions are trivial (all together, or all apart), where
            // agreement carries no information.
            return 0.0;
        }
        (pairs_in_cells - expected) / (max - expected)
    }
}

fn entropy(counts: impl Iterator<Item = usize>, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let total = total as f64;
    -counts
        .filter(|count| *count > 0)
        .map(|count| {
            let share = count as f64 / total;
            share * share.ln()
        })
        .sum::<f64>()
}

fn pairs(count: usize) -> f64 {
    let count = count as f64;
    count * (count - 1.0) / 2.0
}

fn harmonic_mean(left: f64, right: f64) -> f64 {
    if left + right <= 0.0 {
        return 0.0;
    }
    2.0 * left * right / (left + right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assignments(pairs: &[(&str, &str)]) -> Vec<ClusterAssignment> {
        pairs
            .iter()
            .enumerate()
            .map(|(index, (predicted, truth))| ClusterAssignment {
                item: format!("paper-{index}"),
                predicted: (*predicted).to_string(),
                truth: (*truth).to_string(),
            })
            .collect()
    }

    #[test]
    fn an_exact_partition_scores_one_everywhere() {
        let metrics = score(&assignments(&[
            ("Vision", "cs.CV"),
            ("Vision", "cs.CV"),
            ("Language", "cs.CL"),
            ("Language", "cs.CL"),
        ]))
        .expect("metrics");

        assert_eq!(metrics.predicted_clusters, 2);
        assert_eq!(metrics.truth_classes, 2);
        assert!((metrics.purity - 1.0).abs() < 1e-9);
        assert!((metrics.v_measure - 1.0).abs() < 1e-9);
        assert!((metrics.adjusted_rand_index - 1.0).abs() < 1e-9);
    }

    #[test]
    fn folder_names_do_not_have_to_match_label_names() {
        let metrics = score(&assignments(&[
            ("Folder A", "cs.CV"),
            ("Folder A", "cs.CV"),
            ("Folder B", "cs.CL"),
            ("Folder B", "cs.CL"),
        ]))
        .expect("metrics");

        assert!((metrics.adjusted_rand_index - 1.0).abs() < 1e-9);
    }

    #[test]
    fn one_folder_for_everything_is_complete_but_not_homogeneous() {
        let metrics = score(&assignments(&[
            ("All", "cs.CV"),
            ("All", "cs.CV"),
            ("All", "cs.CL"),
            ("All", "cs.CL"),
        ]))
        .expect("metrics");

        assert!(metrics.homogeneity.abs() < 1e-9, "{metrics:?}");
        assert!((metrics.completeness - 1.0).abs() < 1e-9, "{metrics:?}");
        assert!(metrics.adjusted_rand_index.abs() < 1e-9, "{metrics:?}");
    }

    #[test]
    fn a_folder_per_paper_is_homogeneous_but_not_complete() {
        let metrics = score(&assignments(&[
            ("F1", "cs.CV"),
            ("F2", "cs.CV"),
            ("F3", "cs.CL"),
            ("F4", "cs.CL"),
        ]))
        .expect("metrics");

        assert!((metrics.homogeneity - 1.0).abs() < 1e-9, "{metrics:?}");
        assert!(metrics.completeness < 1.0, "{metrics:?}");
        // Purity says 1.0 for the same split, which is why it is never read
        // on its own.
        assert!((metrics.purity - 1.0).abs() < 1e-9);
        assert!(metrics.adjusted_rand_index.abs() < 1e-9, "{metrics:?}");
    }

    #[test]
    fn a_single_misfiled_paper_costs_less_than_a_shuffled_split() {
        let mostly_right = score(&assignments(&[
            ("Vision", "cs.CV"),
            ("Vision", "cs.CV"),
            ("Vision", "cs.CL"),
            ("Language", "cs.CL"),
            ("Language", "cs.CL"),
            ("Language", "cs.CL"),
        ]))
        .expect("metrics");
        let shuffled = score(&assignments(&[
            ("Vision", "cs.CV"),
            ("Language", "cs.CV"),
            ("Vision", "cs.CL"),
            ("Language", "cs.CL"),
            ("Vision", "cs.CL"),
            ("Language", "cs.CL"),
        ]))
        .expect("metrics");

        assert!(
            mostly_right.adjusted_rand_index > shuffled.adjusted_rand_index,
            "one mistake {:.3} should beat a shuffle {:.3}",
            mostly_right.adjusted_rand_index,
            shuffled.adjusted_rand_index
        );
        assert!(shuffled.adjusted_rand_index < 0.1, "{shuffled:?}");
    }

    #[test]
    fn splitting_one_true_class_in_half_is_visible_as_lost_completeness() {
        let metrics = score(&assignments(&[
            ("Vision A", "cs.CV"),
            ("Vision A", "cs.CV"),
            ("Vision B", "cs.CV"),
            ("Vision B", "cs.CV"),
            ("Language", "cs.CL"),
            ("Language", "cs.CL"),
        ]))
        .expect("metrics");

        assert!((metrics.homogeneity - 1.0).abs() < 1e-9, "{metrics:?}");
        assert!(metrics.completeness < 0.85, "{metrics:?}");
        assert!(
            metrics.adjusted_rand_index < 0.6,
            "over-splitting must cost ARI: {metrics:?}"
        );
    }

    #[test]
    fn scoring_nothing_has_no_answer() {
        assert!(score(&[]).is_none());
    }
}
