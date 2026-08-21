//! Folding the model's free-text category labels into countable concepts.
//!
//! Each paper comes back with a category written in prose, so the same concept
//! arrives spelled several ways: different case, plural, word order, or an
//! extra qualifier. Counting the raw strings scatters one concept across many
//! entries with a count of one each, which destroys exactly the frequency
//! signal the taxonomy synthesis needs.
//!
//! Two labels are treated as one concept when their significant words overlap
//! enough. That merges "Speech Recognition" with "Automatic Speech
//! Recognition", while leaving "Learning" and "Deep Learning" apart — a
//! qualifier that halves the shared vocabulary is usually a real distinction.

use std::collections::{BTreeMap, HashSet};

/// Share of significant words two labels must have in common to count as one
/// concept.
const MERGE_THRESHOLD: f32 = 0.6;

/// Words that carry no topical meaning in a category name.
const NOISE_WORDS: [&str; 8] = ["and", "for", "the", "of", "in", "on", "with", "using"];

/// One concept: the spelling to show, and how many papers landed on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelGroup {
    /// Most common original spelling, used wherever the label is shown.
    pub label: String,
    /// Papers across every spelling folded into this group.
    pub count: usize,
    /// Every original spelling, for evidence and debugging.
    pub variants: Vec<String>,
}

/// Groups raw labels into concepts, most frequent first.
///
/// Ordering is deterministic — count descending, then alphabetical — so the
/// same corpus always produces the same synthesis input.
#[must_use]
pub fn group_labels<'a>(labels: impl Iterator<Item = &'a str>) -> Vec<LabelGroup> {
    let mut exact_counts = BTreeMap::<String, usize>::new();
    for label in labels {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            continue;
        }
        *exact_counts.entry(trimmed.to_string()).or_default() += 1;
    }

    // Fold the strongest spellings first so a group's representative is the one
    // most papers actually used.
    let mut ordered = exact_counts.into_iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut groups: Vec<(HashSet<String>, LabelGroup)> = Vec::new();
    for (label, count) in ordered {
        let words = significant_words(&label);
        if words.is_empty() {
            continue;
        }

        match groups
            .iter_mut()
            .find(|(existing, _)| word_overlap(existing, &words) >= MERGE_THRESHOLD)
        {
            Some((_, group)) => {
                group.count += count;
                group.variants.push(label);
            }
            None => groups.push((
                words,
                LabelGroup {
                    label: label.clone(),
                    count,
                    variants: vec![label],
                },
            )),
        }
    }

    let mut grouped = groups
        .into_iter()
        .map(|(_, mut group)| {
            group.variants.sort();
            group
        })
        .collect::<Vec<_>>();
    grouped.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.label.cmp(&right.label))
    });
    grouped
}

/// Meaning-carrying words of a label, normalized for comparison.
///
/// Case, punctuation, word order, and simple plurals all stop mattering here,
/// which is what lets differently written labels meet.
fn significant_words(label: &str) -> HashSet<String> {
    label
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| singular(&word.to_lowercase()))
        .filter(|word| word.len() > 1 && !NOISE_WORDS.contains(&word.as_str()))
        .collect()
}

/// Crude English singular: enough for "networks" and "methods", and harmless
/// on words that only look plural.
fn singular(word: &str) -> String {
    if word.len() > 3 && word.ends_with('s') && !word.ends_with("ss") && !word.ends_with("us") {
        word[..word.len() - 1].to_string()
    } else {
        word.to_string()
    }
}

/// Jaccard overlap of two word sets.
fn word_overlap(left: &HashSet<String>, right: &HashSet<String>) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let shared = left.intersection(right).count() as f32;
    let combined = left.union(right).count() as f32;
    shared / combined
}

/// Splits grouped labels into synthesis batches of related concepts.
///
/// Chunking the list positionally puts unrelated concepts in one request and
/// splits related ones across requests, so each partial taxonomy invents its
/// own branch for the same idea and the merge has to guess they belong
/// together. Seeding each batch with the strongest remaining concept and
/// filling it with its nearest neighbours gives every request a coherent
/// subject instead.
///
/// Batches are still filled to `batch_size` once the related concepts run out:
/// leaving them short would multiply the number of requests, and a few
/// unrelated stragglers cost less than that.
#[must_use]
pub fn batch_related_labels(groups: &[LabelGroup], batch_size: usize) -> Vec<Vec<LabelGroup>> {
    let batch_size = batch_size.max(1);
    let mut remaining = groups.to_vec();
    let mut batches = Vec::new();

    while !remaining.is_empty() {
        let seed = remaining.remove(0);
        let seed_words = significant_words(&seed.label);
        let mut batch = vec![seed];

        while batch.len() < batch_size && !remaining.is_empty() {
            let best = remaining
                .iter()
                .enumerate()
                .map(|(index, candidate)| {
                    let overlap = word_overlap(&seed_words, &significant_words(&candidate.label));
                    (index, overlap)
                })
                .max_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        // Ties fall back to the order they arrived in, which is
                        // strongest concept first.
                        .then_with(|| right.0.cmp(&left.0))
                })
                .map(|(index, _)| index)
                .unwrap_or(0);
            batch.push(remaining.remove(best));
        }

        batches.push(batch);
    }

    batches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grouped(labels: &[&str]) -> Vec<LabelGroup> {
        group_labels(labels.iter().copied())
    }

    #[test]
    fn spelling_differences_stop_splitting_one_concept() {
        let groups = grouped(&[
            "Speech Recognition",
            "speech recognition",
            "Speech  Recognition",
            "Speech Recognition.",
        ]);

        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(groups[0].count, 4);
    }

    #[test]
    fn a_qualifier_that_keeps_most_of_the_words_merges() {
        let groups = grouped(&[
            "Speech Recognition",
            "Speech Recognition",
            "Automatic Speech Recognition",
        ]);

        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(groups[0].count, 3);
        assert_eq!(
            groups[0].label, "Speech Recognition",
            "the most used spelling should represent the group"
        );
    }

    #[test]
    fn a_qualifier_that_changes_the_concept_stays_separate() {
        let groups = grouped(&["Learning", "Deep Learning", "Deep Learning"]);

        assert_eq!(groups.len(), 2, "{groups:?}");
        assert_eq!(groups[0].label, "Deep Learning");
        assert_eq!(groups[0].count, 2);
    }

    #[test]
    fn word_order_and_plurals_do_not_create_new_concepts() {
        let groups = grouped(&["Neural Networks", "Network Neural", "neural network"]);

        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(groups[0].count, 3);
    }

    #[test]
    fn joining_words_are_ignored_when_comparing() {
        let groups = grouped(&[
            "Optimization for Neural Networks",
            "Neural Network Optimization",
        ]);

        assert_eq!(groups.len(), 1, "{groups:?}");
    }

    #[test]
    fn unrelated_labels_are_left_alone() {
        let groups = grouped(&["Quantum Field Theory", "Speech Recognition", "Graph Theory"]);

        assert_eq!(groups.len(), 3, "{groups:?}");
    }

    #[test]
    fn groups_come_back_strongest_first_and_deterministically() {
        let groups = grouped(&[
            "Graph Theory",
            "Speech Recognition",
            "Speech Recognition",
            "Quantum Field Theory",
            "Quantum Field Theory",
            "Quantum Field Theory",
        ]);

        assert_eq!(
            groups.iter().map(|group| group.count).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
        assert_eq!(groups[0].label, "Quantum Field Theory");

        let repeated = grouped(&[
            "Quantum Field Theory",
            "Graph Theory",
            "Speech Recognition",
            "Quantum Field Theory",
            "Speech Recognition",
            "Quantum Field Theory",
        ]);
        assert_eq!(groups, repeated, "order must not depend on input order");
    }

    #[test]
    fn every_original_spelling_is_kept_as_evidence() {
        let groups = grouped(&["Speech Recognition", "speech recognition"]);

        assert_eq!(
            groups[0].variants,
            vec![
                "Speech Recognition".to_string(),
                "speech recognition".to_string()
            ]
        );
    }

    #[test]
    fn related_concepts_share_a_batch_instead_of_being_split_by_position() {
        let groups = grouped(&[
            "Speech Recognition",
            "Speech Recognition",
            "Quantum Field Theory",
            "Quantum Field Theory",
            "Speech Synthesis",
            "Quantum Field Simulation",
        ]);

        let batches = batch_related_labels(&groups, 2);

        let subjects = batches
            .iter()
            .map(|batch| {
                batch
                    .iter()
                    .map(|group| group.label.as_str())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for batch in &subjects {
            let quantum = batch
                .iter()
                .filter(|label| label.contains("Quantum"))
                .count();
            let speech = batch
                .iter()
                .filter(|label| label.contains("Speech"))
                .count();
            assert!(
                quantum == 0 || speech == 0,
                "a batch should hold one subject: {subjects:?}"
            );
        }
    }

    #[test]
    fn batches_are_filled_so_the_request_count_stays_bounded() {
        let groups = grouped(&["Alpha", "Beta", "Gamma", "Delta", "Epsilon"]);

        let batches = batch_related_labels(&groups, 2);

        assert_eq!(batches.len(), 3, "5 labels in batches of 2");
        assert_eq!(batches[0].len(), 2);
        assert_eq!(batches[2].len(), 1, "the remainder forms a short batch");
    }

    #[test]
    fn batching_covers_every_label_exactly_once() {
        let groups = grouped(&[
            "Speech Recognition",
            "Graph Theory",
            "Quantum Field Theory",
            "Neural Networks",
            "Speech Synthesis",
        ]);

        let batches = batch_related_labels(&groups, 2);

        let mut seen = batches
            .iter()
            .flatten()
            .map(|group| group.label.clone())
            .collect::<Vec<_>>();
        seen.sort();
        let mut expected = groups
            .iter()
            .map(|group| group.label.clone())
            .collect::<Vec<_>>();
        expected.sort();
        assert_eq!(seen, expected);
    }

    #[test]
    fn blank_labels_are_dropped() {
        let groups = grouped(&["", "   ", "Graph Theory"]);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label, "Graph Theory");
    }
}
