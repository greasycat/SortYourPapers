//! Comparing ways of grouping papers, on real papers with known labels.
//!
//! The organizer's own grouping needs a model, an API key, and PDFs, so it
//! cannot be measured on demand. The question underneath it — how much
//! grouping signal a paper's text carries, and how much of that survives the
//! pipeline's preprocessing — can be, using nothing but the curated set's
//! abstracts and arithmetic.
//!
//! These strategies are not proposals to ship. They are yardsticks: a run that
//! cannot beat plain term overlap on the same corpus is not earning its model
//! calls.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::error::{AppError, Result};
use crate::eval::{ClusterAssignment, ClusteringMetrics, score};
use crate::llm::{EmbeddingClient, EmbeddingRequest};
use crate::papers::preprocess::preprocess_for_llm;

/// One paper as the bench sees it: some text, and the label it should land
/// under.
#[derive(Debug, Clone)]
pub struct BenchDocument {
    pub id: String,
    pub title: String,
    pub text: String,
    pub truth: String,
}

/// How one grouping strategy scored.
#[derive(Debug, Clone)]
pub struct StrategyResult {
    pub name: &'static str,
    pub clusters: usize,
    pub metrics: ClusteringMetrics,
}

impl StrategyResult {
    #[must_use]
    pub fn table_row(&self) -> String {
        format!(
            "{:<34} {:>3}  ari {:>6.3}  v {:>6.3}  hom {:>6.3}  comp {:>6.3}",
            self.name,
            self.clusters,
            self.metrics.adjusted_rand_index,
            self.metrics.v_measure,
            self.metrics.homogeneity,
            self.metrics.completeness
        )
    }
}

/// Runs every strategy over the same papers and scores each one.
///
/// `target_clusters` is the number of reference labels, given to the
/// strategies that need a stopping point. That is a generous assumption the
/// real pipeline does not get, which is the point: it bounds what the text
/// alone can support.
#[must_use]
pub fn run_bench(documents: &[BenchDocument], target_clusters: usize) -> Vec<StrategyResult> {
    if documents.is_empty() {
        return Vec::new();
    }

    let mut results = Vec::new();

    results.push(evaluate(
        "everything in one folder",
        documents,
        &vec![0; documents.len()],
    ));
    results.push(evaluate(
        "one folder per paper",
        documents,
        &(0..documents.len()).collect::<Vec<_>>(),
    ));

    let abstract_terms = documents
        .iter()
        .map(|document| tokenize(&document.text))
        .collect::<Vec<_>>();
    results.push(evaluate(
        "abstract terms, k = labels",
        documents,
        &agglomerate(&tfidf(&abstract_terms), target_clusters),
    ));

    // The same text after the reduction the pipeline applies before any model
    // sees it, which shows what that reduction costs.
    let reduced_terms = documents
        .iter()
        .map(|document| tokenize(&preprocess_for_llm(&document.text)))
        .collect::<Vec<_>>();
    results.push(evaluate(
        "pipeline-reduced terms, k = labels",
        documents,
        &agglomerate(&tfidf(&reduced_terms), target_clusters),
    ));

    let title_terms = documents
        .iter()
        .map(|document| tokenize(&document.title))
        .collect::<Vec<_>>();
    results.push(evaluate(
        "title terms, k = labels",
        documents,
        &agglomerate(&tfidf(&title_terms), target_clusters),
    ));

    let title_and_abstract = documents
        .iter()
        .map(|document| tokenize(&format!("{} {}", document.title, document.text)))
        .collect::<Vec<_>>();
    results.push(evaluate(
        "title + abstract, k = labels",
        documents,
        &agglomerate(&tfidf(&title_and_abstract), target_clusters),
    ));

    // Word pairs, which carry the distinctions single words lose: "neural
    // network" and "network protocol" share a word but not a subject.
    let bigram_terms = documents
        .iter()
        .map(|document| with_bigrams(&tokenize(&format!("{} {}", document.title, document.text))))
        .collect::<Vec<_>>();
    results.push(evaluate(
        "words + word pairs, k = labels",
        documents,
        &agglomerate(&tfidf(&bigram_terms), target_clusters),
    ));

    // Keeping only the rarest terms per paper, on the theory that the shared
    // middle of the vocabulary is what blurs the fields together.
    let distinctive_terms = most_distinctive(&title_and_abstract, 12);
    results.push(evaluate(
        "12 rarest terms, k = labels",
        documents,
        &agglomerate(&tfidf(&distinctive_terms), target_clusters),
    ));

    // Without the label count, a strategy has to decide when to stop merging.
    results.push(evaluate(
        "title + abstract, k unknown",
        documents,
        &agglomerate_until(&tfidf(&title_and_abstract), 0.12),
    ));

    results
}

/// Groups papers by the embedding of their text rather than by shared words.
///
/// Term overlap cannot see that "quark" and "hadron" belong together; an
/// embedding can. This is the one candidate the offline bench cannot settle on
/// its own, because it needs a provider call.
///
/// # Errors
/// Returns an error when the embedding request fails or comes back short.
pub async fn bench_embeddings(
    documents: &[BenchDocument],
    client: &dyn EmbeddingClient,
    target_clusters: usize,
) -> Result<StrategyResult> {
    let request = EmbeddingRequest::from_texts(
        documents
            .iter()
            .map(|document| format!("{} {}", document.title, document.text)),
    );
    let response = client.embed(&request).await?;
    if response.embeddings.len() != documents.len() {
        return Err(AppError::Validation(format!(
            "embedding count {} did not match {} document(s)",
            response.embeddings.len(),
            documents.len()
        )));
    }

    // Reuse the same clustering the term strategies use, so the only thing
    // that changes is how similarity is measured.
    let vectors = response
        .embeddings
        .iter()
        .map(|embedding| {
            embedding
                .values
                .iter()
                .enumerate()
                .map(|(index, value)| (format!("{index:05}"), f64::from(*value)))
                .collect::<BTreeMap<_, _>>()
        })
        .map(normalize)
        .collect::<Vec<_>>();

    Ok(evaluate(
        "embeddings, k = labels",
        documents,
        &agglomerate(&vectors, target_clusters),
    ))
}

fn normalize(mut vector: BTreeMap<String, f64>) -> BTreeMap<String, f64> {
    let norm = vector
        .values()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    if norm > 0.0 {
        for value in vector.values_mut() {
            *value /= norm;
        }
    }
    vector
}

/// Adds adjacent word pairs alongside the single words.
fn with_bigrams(terms: &[String]) -> Vec<String> {
    let mut expanded = terms.to_vec();
    for pair in terms.windows(2) {
        expanded.push(format!("{}_{}", pair[0], pair[1]));
    }
    expanded
}

/// Keeps each paper's rarest terms, measured across the whole corpus.
fn most_distinctive(documents: &[Vec<String>], keep: usize) -> Vec<Vec<String>> {
    let mut document_frequency = HashMap::<&str, usize>::new();
    for terms in documents {
        for term in terms.iter().collect::<HashSet<_>>() {
            *document_frequency.entry(term.as_str()).or_insert(0) += 1;
        }
    }

    documents
        .iter()
        .map(|terms| {
            let mut unique = terms.iter().cloned().collect::<Vec<_>>();
            unique.sort();
            unique.dedup();
            unique.sort_by_key(|term| {
                (
                    *document_frequency.get(term.as_str()).unwrap_or(&1),
                    term.clone(),
                )
            });
            unique.into_iter().take(keep).collect()
        })
        .collect()
}

fn evaluate(name: &'static str, documents: &[BenchDocument], labels: &[usize]) -> StrategyResult {
    let assignments = documents
        .iter()
        .zip(labels.iter())
        .map(|(document, cluster)| ClusterAssignment {
            item: document.id.clone(),
            predicted: format!("cluster-{cluster}"),
            truth: document.truth.clone(),
        })
        .collect::<Vec<_>>();

    StrategyResult {
        name,
        clusters: labels.iter().collect::<HashSet<_>>().len(),
        metrics: score(&assignments).unwrap_or_else(|| {
            unreachable!("documents are non-empty when a strategy is evaluated")
        }),
    }
}

/// Meaning-carrying words, lowercased and de-pluralized.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.len() > 2)
        .map(|word| {
            let lowered = word.to_lowercase();
            if lowered.len() > 3 && lowered.ends_with('s') && !lowered.ends_with("ss") {
                lowered[..lowered.len() - 1].to_string()
            } else {
                lowered
            }
        })
        .filter(|word| !STOP_WORDS.contains(&word.as_str()))
        .collect()
}

const STOP_WORDS: [&str; 30] = [
    "the", "and", "for", "that", "with", "this", "from", "are", "can", "our", "which", "have",
    "has", "not", "but", "all", "use", "using", "used", "new", "such", "these", "their", "than",
    "them", "then", "there", "when", "where", "while",
];

/// Term-frequency times inverse document frequency, as unit-length vectors.
///
/// Weighting by rarity is what stops shared academic boilerplate — "results",
/// "method", "propose" — from making every paper look alike.
fn tfidf(documents: &[Vec<String>]) -> Vec<BTreeMap<String, f64>> {
    let total = documents.len() as f64;
    let mut document_frequency = HashMap::<&str, usize>::new();
    for terms in documents {
        for term in terms.iter().collect::<HashSet<_>>() {
            *document_frequency.entry(term.as_str()).or_insert(0) += 1;
        }
    }

    documents
        .iter()
        .map(|terms| {
            let mut counts = BTreeMap::<String, f64>::new();
            for term in terms {
                *counts.entry(term.clone()).or_insert(0.0) += 1.0;
            }

            let mut weighted = counts
                .into_iter()
                .map(|(term, count)| {
                    let seen_in = *document_frequency.get(term.as_str()).unwrap_or(&1) as f64;
                    let weight = (1.0 + count.ln()) * (total / seen_in).ln();
                    (term, weight)
                })
                .collect::<BTreeMap<_, _>>();

            let norm = weighted
                .values()
                .map(|weight| weight * weight)
                .sum::<f64>()
                .sqrt();
            if norm > 0.0 {
                for weight in weighted.values_mut() {
                    *weight /= norm;
                }
            }
            weighted
        })
        .collect()
}

fn cosine(left: &BTreeMap<String, f64>, right: &BTreeMap<String, f64>) -> f64 {
    let (small, large) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    small
        .iter()
        .filter_map(|(term, weight)| large.get(term).map(|other| weight * other))
        .sum()
}

/// Average-linkage agglomerative clustering, stopping at `target` clusters.
fn agglomerate(vectors: &[BTreeMap<String, f64>], target: usize) -> Vec<usize> {
    merge_clusters(vectors, target.max(1), f64::NEG_INFINITY)
}

/// The same, stopping when nothing is similar enough to merge.
fn agglomerate_until(vectors: &[BTreeMap<String, f64>], min_similarity: f64) -> Vec<usize> {
    merge_clusters(vectors, 1, min_similarity)
}

fn merge_clusters(
    vectors: &[BTreeMap<String, f64>],
    target: usize,
    min_similarity: f64,
) -> Vec<usize> {
    let mut membership = (0..vectors.len())
        .map(|index| vec![index])
        .collect::<Vec<_>>();

    while membership.len() > target {
        let mut best: Option<(usize, usize, f64)> = None;
        for left in 0..membership.len() {
            for right in (left + 1)..membership.len() {
                let similarity = average_linkage(vectors, &membership[left], &membership[right]);
                if best.is_none_or(|(_, _, current)| similarity > current) {
                    best = Some((left, right, similarity));
                }
            }
        }

        let Some((left, right, similarity)) = best else {
            break;
        };
        if similarity < min_similarity {
            break;
        }

        let merged = membership.remove(right);
        membership[left].extend(merged);
    }

    let mut labels = vec![0; vectors.len()];
    for (cluster, members) in membership.iter().enumerate() {
        for member in members {
            labels[*member] = cluster;
        }
    }
    labels
}

fn average_linkage(vectors: &[BTreeMap<String, f64>], left: &[usize], right: &[usize]) -> f64 {
    let mut total = 0.0;
    for first in left {
        for second in right {
            total += cosine(&vectors[*first], &vectors[*second]);
        }
    }
    total / (left.len() * right.len()) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(id: &str, title: &str, text: &str, truth: &str) -> BenchDocument {
        BenchDocument {
            id: id.to_string(),
            title: title.to_string(),
            text: text.to_string(),
            truth: truth.to_string(),
        }
    }

    #[test]
    fn clearly_separated_topics_are_recovered_from_text_alone() {
        let documents = vec![
            document(
                "1",
                "Convolutional networks for image recognition",
                "image recognition convolutional network vision pixels",
                "vision",
            ),
            document(
                "2",
                "Deep residual image classification",
                "image classification convolutional vision pixels residual",
                "vision",
            ),
            document(
                "3",
                "Quark confinement in lattice gauge theory",
                "quark lattice gauge boson confinement hadron",
                "physics",
            ),
            document(
                "4",
                "Hadron spectra from lattice simulations",
                "hadron lattice gauge quark spectra simulation",
                "physics",
            ),
        ];

        let results = run_bench(&documents, 2);
        let by_terms = results
            .iter()
            .find(|result| result.name == "title + abstract, k = labels")
            .expect("strategy");

        assert!(
            (by_terms.metrics.adjusted_rand_index - 1.0).abs() < 1e-9,
            "term overlap should separate two unrelated fields: {by_terms:?}"
        );
    }

    #[test]
    fn the_floors_behave_as_floors() {
        let documents = vec![
            document("1", "a", "alpha beta", "x"),
            document("2", "b", "alpha beta", "x"),
            document("3", "c", "gamma delta", "y"),
            document("4", "d", "gamma delta", "y"),
        ];

        let results = run_bench(&documents, 2);
        let single = results
            .iter()
            .find(|result| result.name == "everything in one folder")
            .expect("strategy");
        let per_paper = results
            .iter()
            .find(|result| result.name == "one folder per paper")
            .expect("strategy");

        assert_eq!(single.clusters, 1);
        assert!(single.metrics.adjusted_rand_index.abs() < 1e-9);
        assert_eq!(per_paper.clusters, 4);
        assert!(per_paper.metrics.adjusted_rand_index.abs() < 1e-9);
    }

    #[test]
    fn rare_words_outweigh_boilerplate() {
        let shared = "we propose a method and report results on a benchmark";
        let documents = vec![
            document("1", "t", &format!("{shared} quark lattice"), "physics"),
            document("2", "t", &format!("{shared} quark lattice"), "physics"),
            document("3", "t", &format!("{shared} pixels convolution"), "vision"),
            document("4", "t", &format!("{shared} pixels convolution"), "vision"),
        ];

        let results = run_bench(&documents, 2);
        let by_terms = results
            .iter()
            .find(|result| result.name == "abstract terms, k = labels")
            .expect("strategy");

        assert!(
            by_terms.metrics.adjusted_rand_index > 0.9,
            "boilerplate shared by every paper must not dominate: {by_terms:?}"
        );
    }

    /// Returns embeddings that place the two subjects on opposite axes, which is
    /// what a real embedding model does for unrelated fields.
    struct StubEmbeddingClient;

    #[async_trait::async_trait]
    impl EmbeddingClient for StubEmbeddingClient {
        async fn embed(
            &self,
            request: &EmbeddingRequest,
        ) -> syp_ai::error::Result<crate::llm::EmbeddingResponse> {
            Ok(crate::llm::EmbeddingResponse {
                embeddings: request
                    .inputs
                    .iter()
                    .map(|input| crate::llm::EmbeddingVector {
                        values: if input.text.contains("quark") {
                            vec![1.0, 0.0]
                        } else {
                            vec![0.0, 1.0]
                        },
                    })
                    .collect(),
                metrics: crate::llm::LlmCallMetrics::default(),
            })
        }
    }

    #[tokio::test]
    async fn the_embedding_strategy_groups_by_meaning_not_shared_words() {
        let documents = vec![
            document("1", "Confinement", "quark lattice gauge", "physics"),
            document("2", "Spectra", "quark hadron simulation", "physics"),
            document("3", "Recognition", "image pixels convolution", "vision"),
            document("4", "Detection", "image bounding boxes", "vision"),
        ];

        let result = bench_embeddings(&documents, &StubEmbeddingClient, 2)
            .await
            .expect("embedding bench");

        assert_eq!(result.clusters, 2);
        assert!(
            (result.metrics.adjusted_rand_index - 1.0).abs() < 1e-9,
            "{result:?}"
        );
    }

    #[test]
    fn an_empty_corpus_produces_no_results() {
        assert!(run_bench(&[], 3).is_empty());
    }
}

/// Runs the bench over the shipped curated set and prints the comparison.
///
/// Ignored by default because it is a maintainer's measurement, not a
/// regression check: run it with
/// `cargo test -p syp-library curated_set_clustering_bench -- --ignored --nocapture`.
#[cfg(test)]
mod curated_set_bench {
    use std::collections::BTreeSet;

    use crate::testsets::load_manifest_from_path;

    use super::*;

    fn documents(label: fn(&crate::testsets::CuratedPaper) -> String) -> Vec<BenchDocument> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/testsets/scijudgebench-diverse.toml");
        load_manifest_from_path(&path)
            .expect("shipped manifest")
            .papers
            .iter()
            .map(|paper| BenchDocument {
                id: paper.paper_id.clone(),
                title: paper.title.clone(),
                text: paper.abstract_excerpt.clone(),
                truth: label(paper),
            })
            .collect()
    }

    /// Scores embedding-based grouping against the term strategies.
    ///
    /// The only bench entry that needs a provider, so it is gated on a key
    /// being present rather than on the maintainer remembering a flag.
    #[tokio::test]
    #[ignore = "needs SYP_API_KEY and network; run explicitly with --nocapture"]
    async fn curated_set_embedding_bench() {
        let api_key = std::env::var("SYP_API_KEY")
            .expect("set SYP_API_KEY to score embedding-based grouping");
        let provider = std::env::var("SYP_EMBEDDING_PROVIDER").unwrap_or_else(|_| "gemini".into());
        let model = std::env::var("SYP_EMBEDDING_MODEL")
            .unwrap_or_else(|_| "gemini-embedding-2-preview".into());

        let client = crate::llm::build_embedding_client(&crate::llm::EmbeddingConfig {
            provider: match provider.as_str() {
                "openai" => crate::llm::LlmProvider::Openai,
                "ollama" => crate::llm::LlmProvider::Ollama,
                _ => crate::llm::LlmProvider::Gemini,
            },
            model,
            base_url: std::env::var("SYP_EMBEDDING_BASE_URL").ok(),
            api_key: Some(api_key),
        })
        .expect("embedding client");

        let docs = documents(|paper| paper.category.clone())
            .into_iter()
            .filter(|document| document.truth != "Others")
            .collect::<Vec<_>>();
        let classes = docs
            .iter()
            .map(|document| document.truth.clone())
            .collect::<BTreeSet<_>>()
            .len();

        println!(
            "\n=== embeddings vs terms, {} papers, {classes} label(s) ===",
            docs.len()
        );
        for result in run_bench(&docs, classes) {
            println!("{}", result.table_row());
        }
        let embedded = bench_embeddings(&docs, client.as_ref(), classes)
            .await
            .expect("embedding bench");
        println!("{}", embedded.table_row());
    }

    /// Sweeps the stopping threshold for the strategy that is not told how many
    /// labels exist, which is the situation the real pipeline is in.
    #[test]
    #[ignore = "measurement over the curated set; run explicitly with --nocapture"]
    fn curated_set_threshold_sweep() {
        let docs = documents(|paper| paper.category.clone())
            .into_iter()
            .filter(|document| document.truth != "Others")
            .collect::<Vec<_>>();
        let vectors = tfidf(
            &docs
                .iter()
                .map(|document| tokenize(&preprocess_for_llm(&document.text)))
                .collect::<Vec<_>>(),
        );

        println!(
            "\n=== stopping threshold sweep, {} papers, coherent fields only ===",
            docs.len()
        );
        for step in 1..=12 {
            let threshold = f64::from(step) * 0.02;
            let labels = agglomerate_until(&vectors, threshold);
            let result = evaluate("sweep", &docs, &labels);
            println!(
                "min-similarity {:.2}  clusters {:>3}  ari {:>6.3}  v {:>6.3}",
                threshold,
                result.clusters,
                result.metrics.adjusted_rand_index,
                result.metrics.v_measure
            );
        }
    }

    #[test]
    #[ignore = "measurement over the curated set; run explicitly with --nocapture"]
    fn curated_set_clustering_bench() {
        for (granularity, label) in [
            (
                "category",
                (|paper: &crate::testsets::CuratedPaper| paper.category.clone())
                    as fn(&_) -> String,
            ),
            ("subcategory", |paper: &crate::testsets::CuratedPaper| {
                paper.subcategory.clone()
            }),
        ] {
            let docs = documents(label);
            let classes = docs
                .iter()
                .map(|document| document.truth.clone())
                .collect::<BTreeSet<_>>()
                .len();

            println!(
                "\n=== grouping {} papers against {classes} {granularity} label(s) ===",
                docs.len()
            );
            for result in run_bench(&docs, classes) {
                println!("{}", result.table_row());
            }

            // "Others" is a grab-bag with no shared subject, so it cannot be
            // grouped by any method; measuring without it shows what the
            // coherent fields actually support.
            if granularity == "category" {
                let coherent = docs
                    .iter()
                    .filter(|document| document.truth != "Others")
                    .cloned()
                    .collect::<Vec<_>>();
                let coherent_classes = coherent
                    .iter()
                    .map(|document| document.truth.clone())
                    .collect::<BTreeSet<_>>()
                    .len();
                println!(
                    "\n=== same, excluding the Others grab-bag: {} papers, {coherent_classes} label(s) ===",
                    coherent.len()
                );
                for result in run_bench(&coherent, coherent_classes) {
                    println!("{}", result.table_row());
                }
            }
        }
    }
}
